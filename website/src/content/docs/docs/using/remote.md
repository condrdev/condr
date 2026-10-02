---
title: Remote connections
description: Learn how to set up a Headless Server and connect securely to remote devices over SSH, TCP or P2P, keeping your Agents running steadily.
---

Condr lets you manage Agent tasks across machines from a single desktop window. With the work running on a remote device, your Agents keep working without interruption, unaffected by restarts or network drops on your local workstation.

## Persistent background operation

Condr uses a split Client/Server architecture:

- **Server**: the `condr` command-line program. It runs on the target device, manages terminal processes and keeps Agents running in the background.
- **Client**: the `condr-gui` desktop app. It connects to one or more Servers and provides the graphical interface.

In persistent background mode:

- Closing the desktop Client window does not stop the remote Server or the Agent tasks that are running.
- During a brief network outage, the Server keeps its current state and continues running tasks in the background.
- When you reopen the Client and reconnect, Condr automatically syncs the latest terminal output and Agent states.

## Prerequisite: install the Headless Server

Before connecting to a remote device, install the Headless Server (the `condr` command-line tool, with no graphical interface) on that device.

If that device already has the desktop app installed, you don't need to install anything else: the desktop app ships with a Server that starts automatically when you open the app and keeps running after you close the window. Only machines without a graphical interface need the commands below.

### Install commands

In a terminal, run the install command for the remote device's operating system:

| Operating system | Install command |
| :--- | :--- |
| **Linux** (x86_64 / arm64) | `curl -fsSL https://condr.dev/install.sh \| sh` |
| **macOS** (Apple Silicon / Intel) | `curl -fsSL https://condr.dev/install.sh \| sh` |
| **Windows** (x86_64) | `irm https://condr.dev/install.ps1 \| iex` |

After installation, run `condr --version` in the remote terminal to confirm it succeeded.

## Ways to connect to a remote device

Condr supports three connection protocols to fit different network topologies. Click **Connect Remote Device** in the Client sidebar to start connecting.

### SSH connection

For remote servers or cloud hosts you can already reach with OpenSSH.

- **When to use**: servers you can already log in to over SSH, with a key pair set up.
- **Steps**:
  1. In the Client, click **Connect Remote Device** → **SSH**.
  2. Enter an address that starts with `ssh://`, for example:
     - `ssh://user@192.168.1.100`
     - `ssh://my-cloud-server` (host aliases in your local `~/.ssh/config` are read automatically)
  3. Confirm the connection. Condr uses your local OpenSSH configuration to set up an encrypted channel, and automatically starts or reuses the remote `condr` Server.

### P2P connection

For networks where both sides sit behind NAT or a firewall and have no fixed public IP.

- **When to use**: office computers on different networks, home development machines, or complex network setups.
- **Full procedure**:
  1. **Start P2P mode**: On the remote device, start the Server in Peer-to-peer mode; this setting is written to the config. If the Server is already running, replace `start` with `restart`:

     ```sh
     condr server start --p2p
     ```

  2. **Get a P2P link**: Run `condr server invite`. It prints a link that starts with `p2p://` and needs no address. The link is valid for 10 minutes and can be used only once.
  3. **Connect from the Client**: In the Client, click **Connect Remote Device** → **Peer-to-peer** and paste the `p2p://` link.
  4. **Establish the connection**: Condr tries a direct peer-to-peer connection first. If the network prevents a direct connection, it automatically forwards data through an encrypted relay.

### TCP connection

For servers with a fixed IP address, or ones directly reachable on an internal LAN. All traffic is protected end to end with `Noise_IKpsk2` encryption.

- **When to use**: devices with a static IP or on the same LAN, in environments that don't rely on an SSH service.
- **Full procedure**:
  1. **Enable listening**: By default the Server listens only on the local machine. On the remote device, start it with a listen address. The address is written to the config, so later starts don't need it again. If the Server is already running, replace `start` with `restart`:

     ```sh
     condr server start --listen 0.0.0.0:2637
     ```

  2. **Generate a pairing invite**: On the remote device, run the following command to generate a one-time pairing link:

     ```sh
     condr server invite
     ```

  3. **Copy the pairing link**: The console prints a link in the form `tcp://<device key>.<invite>@<host>:<port>`. Replace `<host>` with this device's IP. The link is valid for 10 minutes and can be used only once.
  4. **Connect from the Client**: In the Condr Client, click **Connect Remote Device** → **TCP**, paste the link and confirm.
  5. **Complete the key handshake**: The Client and Server set up a secure point-to-point channel using the pre-shared key. Once pairing completes, Condr stores only the Device key and the address, not the invite.

## Troubleshooting

### SSH reports condr: not found

**Symptom**: When connecting over SSH, the Client shows the error `bash: condr: not found` or `command not found`.

**Cause**: The install script puts `condr` in `~/.local/opt/condr`, creates a symlink in `~/.local/bin`, and writes the PATH setting for that directory into `~/.profile` or `~/.zprofile`. Non-interactive SSH sessions usually don't load either file, so the system can't find the `condr` executable.

**Solutions**:

**Option 1: Specify the path in the connection address (recommended)**. Without changing anything on the remote device, tell Condr where the executable is right in the SSH address:

```text
ssh://user@host?bin=/home/user/.local/bin/condr
```

**Option 2: Set the environment variable at the top of the shell startup file**:

1. Log in to the remote device and check where `condr` is installed:

   ```sh
   which condr
   ```

2. Edit `~/.bashrc` or `~/.zshenv` on the remote device.
3. Put the PATH setting at the very top of the file (it must come before the code that returns early for non-interactive shells):

   ```sh
   export PATH="$HOME/.local/bin:$PATH"
   ```

**Option 3: Create a system-wide symlink**. On the remote device, symlink `condr` into a standard system executable path:

```sh
sudo ln -s ~/.local/bin/condr /usr/local/bin/condr
```

### TCP connection times out or fails to handshake

If a TCP connection keeps timing out, check the following:

1. **Check that the port is open**: Make sure the remote device's firewall and your cloud provider's security group allow the TCP listening port.
2. **Check that the invite is still valid**: A pairing link generated by `condr server invite` expires after 10 minutes, and it also stops working after one use. If it has expired, generate a new one and enter the new link in the Client.
3. **Check the background service**: On the remote device, run `condr server status` and confirm that the Server process is running and that the Listen line shows the listen address.
