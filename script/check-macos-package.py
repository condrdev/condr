#!/usr/bin/env python3
"""Check PATH registration; optionally install PACKAGE.pkg into the current user home."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile


repo = Path(__file__).resolve().parent.parent
postinstall = repo / "packaging/macos/scripts/postinstall"
marker = "# condr user bin"
shells = ["/bin/bash"]
if Path("/bin/zsh").exists():
    shells.append("/bin/zsh")

with tempfile.TemporaryDirectory(prefix="condr-macos-check-") as temporary:
    stage = Path(temporary)
    wrong_home = stage / "installer-home"
    wrong_home.mkdir()
    for existing in [(), (".profile",), (".bash_login", ".profile"),
                     (".bash_profile", ".bash_login", ".profile")]:
        target = stage / f'user {len(existing)} "quoted" $dollar'
        binary = target / "Applications/Condr.app/Contents/MacOS/condr"
        binary.parent.mkdir(parents=True)
        binary.write_text("#!/bin/sh\nexit 0\n")
        binary.chmod(0o755)
        entry = target / ".local/bin/condr"
        entry.parent.mkdir(parents=True)
        entry.symlink_to("../../Applications/Condr.app/Contents/MacOS/condr")
        original = "# existing settings\nexport CONDR_PROFILE_PRESERVED=yes"
        existing_profiles = (".zprofile", *existing) if existing else ()
        for name in existing_profiles:
            (target / name).write_text(original)
            (target / name).chmod(0o600)

        command = [str(postinstall), "Condr.pkg", str(target), "/"]
        env = {**os.environ, "HOME": str(wrong_home)}
        subprocess.run(command, env=env, check=True, stdout=subprocess.DEVNULL)
        profiles = list(target.glob(".*profile")) + list(target.glob(".bash_login"))
        contents = {path: path.read_bytes() for path in profiles}
        subprocess.run(command, env=env, check=True, stdout=subprocess.DEVNULL)
        assert contents == {path: path.read_bytes() for path in profiles}
        selected = existing[0] if existing else ".profile"
        for path in profiles:
            active = path.name in (".zprofile", selected)
            assert path.read_text().count(marker) == int(active), path
            if path.name in existing_profiles:
                assert path.read_text().startswith(original), path
                assert path.stat().st_mode & 0o777 == 0o600, path
            if active:
                for shell in shells:
                    result = subprocess.run(
                        [shell, "-c", '. "$1"; . "$1"; command -v condr; printf "%s\\n" "$PATH"',
                         "check", str(path)],
                        env={"HOME": str(target), "PATH": "/usr/bin:/bin"},
                        check=True, capture_output=True, text=True,
                    )
                    resolved, path_value = result.stdout.splitlines()
                    assert resolved == str(entry), resolved
                    assert path_value.split(":").count(str(entry.parent)) == 1
        assert not list(wrong_home.iterdir())

    result = subprocess.run(
        [str(postinstall), "Condr.pkg", str(wrong_home), "/"],
        capture_output=True,
    )
    assert result.returncode != 0
    assert not list(wrong_home.iterdir())

print("macOS PATH registration checks passed.")

if len(sys.argv) > 1:
    if sys.platform != "darwin" or len(sys.argv) != 2:
        sys.exit("Pass exactly one .pkg on macOS to check native installation.")
    package = Path(sys.argv[1]).resolve(strict=True)
    installed_home = Path.home()
    snapshots = []
    for _ in range(2):
        subprocess.run(
            ["/usr/sbin/installer", "-pkg", str(package), "-target", "CurrentUserHomeDirectory"],
            check=True,
        )
        snapshots.append({name: (installed_home / name).read_bytes()
                          for name in (".zprofile", ".bash_profile", ".bash_login", ".profile")
                          if (installed_home / name).is_file()})
        for shell in shells:
            subprocess.run(
                [shell, "-lc", 'test "$(command -v condr)" = "$HOME/.local/bin/condr" && condr server --help'],
                env={"HOME": str(installed_home), "PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
                check=True,
            )
    assert snapshots[0] == snapshots[1], "Reinstall changed shell profiles"
    print("macOS package installation and login-shell checks passed.")
