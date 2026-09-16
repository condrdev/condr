#!/usr/bin/env node
// Regenerates the Files sidebar's file-type icons from a checkout of
// material-icon-theme (https://github.com/material-extensions/vscode-material-icon-theme,
// MIT): copies the SVGs the mappings refer to into crates/condr-gui/assets/icons/material
// and writes the lookup tables to crates/condr-gui/src/app/file_icons/generated.rs.
//
//   node script/generate-file-icons.mjs /path/to/vscode-material-icon-theme
//
// What is taken: the `specific` folder theme and every file icon without an `enabledFor`
// icon pack (those belong to framework packs the user picks in VS Code). Light-theme
// variants are left out; the dark icons are drawn in both modes. Folder "open" variants
// are not generated: the chevron already says a directory is unfolded.

import { execSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const source = process.argv[2];
if (!source) {
  console.error("usage: generate-file-icons.mjs <material-icon-theme checkout>");
  process.exit(2);
}
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const assetsDir = join(repo, "crates/condr-gui/assets/icons/material");
const generated = join(repo, "crates/condr-gui/src/app/file_icons/generated.rs");

// The TS mapping files are object literals plus a few imports; strip the imports and the
// type annotations, then evaluate them with stubs for what they import.
function evaluate(file, exportName) {
  let code = readFileSync(join(source, "src/core/icons", file), "utf8");
  code = code.replace(/^import[^;]*;$/gm, "");
  code = code.replace(new RegExp(`export const ${exportName}: [^=]+=`), `globalThis.${exportName} =`);
  const patterns = {
    ecmascript: (name) => ["js", "mjs", "cjs", "ts", "mts", "cts"].map((ext) => `${name}.${ext}`),
    configuration: (name) => ["json", "jsonc", "json5", "yaml", "yml", "toml"].map((ext) => `${name}.${ext}`),
    nodeEcosystem: (name) => [...patterns.ecmascript(name), ...patterns.configuration(name)],
    cosmiconfig: (name) => [
      `.${name}rc`,
      ...patterns.nodeEcosystem(`.${name}rc`),
      ...patterns.nodeEcosystem(`${name}.config`),
    ],
    yaml: (name) => [`${name}.yaml`, `${name}.yml`],
    dotfile: (name) => [`.${name}`, name],
  };
  const context = {
    globalThis: {},
    IconPack: new Proxy({}, { get: (_, key) => String(key) }),
    FileNamePattern: {
      Ecmascript: "ecmascript",
      Configuration: "configuration",
      NodeEcosystem: "nodeEcosystem",
      Cosmiconfig: "cosmiconfig",
      Yaml: "yaml",
      Dotfile: "dotfile",
    },
    parseByPattern: (icons) =>
      icons.map((icon) => {
        if (!icon.patterns) return icon;
        const names = Object.entries(icon.patterns).flatMap(([name, pattern]) => patterns[pattern](name));
        return { ...icon, fileNames: [...(icon.fileNames ?? []), ...names] };
      }),
  };
  context.globalThis = context;
  vm.runInNewContext(code, context, { filename: file });
  return context[exportName];
}

const fileIcons = evaluate("fileIcons.ts", "fileIcons");
const folderTheme = evaluate("folderIcons.ts", "folderIcons").find((theme) => theme.name === "specific");

// Drawn by this script rather than copied; see below.
const synthesized = new Set(["file", "folder"]);
const available = new Set(readdirSync(join(source, "icons")).map((name) => name.replace(/\.svg$/, "")));
const used = new Set([fileIcons.defaultIcon.name, folderTheme.defaultIcon.name]);
// First definition wins, as VS Code resolves them.
const insert = (map, key, icon) => {
  key = key.toLowerCase();
  if (key.includes("/") || map.has(key) || !(available.has(icon) || synthesized.has(icon))) return;
  map.set(key, icon);
  used.add(icon);
};
const fileNames = new Map();
const fileExtensions = new Map();
const folderNames = new Map();
for (const icon of fileIcons.icons) {
  if (icon.enabledFor) continue;
  for (const name of icon.fileNames ?? []) insert(fileNames, name, icon.name);
  for (const ext of icon.fileExtensions ?? []) insert(fileExtensions, ext, icon.name);
}
for (const icon of folderTheme.icons) {
  if (icon.enabledFor) continue;
  for (const name of icon.folderNames ?? []) insert(folderNames, name, icon.name);
}

rmSync(assetsDir, { recursive: true, force: true });
mkdirSync(assetsDir, { recursive: true });
// The theme draws for VS Code's brighter chrome and glares on Condr's dark panels; this
// is the theme's own "saturation" setting, applied the way its build applies it: a
// saturate filter on the root element (src/core/generator/iconSaturation.ts). Opacity is
// applied at draw time instead, since it depends on the theme mode.
const SATURATION = 0.75;
const desaturate = (svg) =>
  svg
    .replace(/^(\s*<svg)(?![^>]*\sfilter=)/, `$1 filter="url(#saturation)"`)
    .replace(/<\/svg>\s*$/, `<filter id="saturation"><feColorMatrix type="saturate" values="${SATURATION}"/></filter></svg>\n`);

const icons = [...used].sort();
for (const icon of icons.filter((icon) => available.has(icon))) {
  const svg = readFileSync(join(source, "icons", `${icon}.svg`), "utf8");
  writeFileSync(join(assetsDir, `${icon}.svg`), desaturate(svg));
}

// The default file and folder icons are not in the repository's icons directory: the
// extension draws them at build time from these paths (src/core/generator/fileGenerator.ts
// and folderGenerator.ts) in its default blue-grey, and so does this script.
const defaultColor = "#90a4ae";
const defaultPaths = {
  file: "m8.668 6h3.6641l-3.6641-3.668v3.668m-4.668-4.668h5.332l4 4v8c0 0.73828-0.59375 1.3359-1.332 1.3359h-8c-0.73828 0-1.332-0.59766-1.332-1.3359v-10.664c0-0.74219 0.59375-1.3359 1.332-1.3359m3.332 1.3359h-3.332v10.664h8v-6h-4.668z",
  folder: "m6.922 3.768-.644-.536A1 1 0 0 0 5.638 3H2a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V5a1 1 0 0 0-1-1H7.562a1 1 0 0 1-.64-.232",
};
for (const [name, path] of Object.entries(defaultPaths)) {
  if (available.has(name)) continue;
  writeFileSync(
    join(assetsDir, `${name}.svg`),
    desaturate(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="${defaultColor}" d="${path}"/></svg>\n`),
  );
  if (!icons.includes(name)) icons.push(name);
}
icons.sort();

const commit = execSync("git rev-parse --short HEAD", { cwd: source }).toString().trim();
const version = JSON.parse(readFileSync(join(source, "package.json"), "utf8")).version;
const table = (name, map) =>
  `pub(super) static ${name}: &[(&str, &str)] = &[\n${[...map]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([key, icon]) => `    (${JSON.stringify(key)}, ${JSON.stringify(icon)}),`)
    .join("\n")}\n];\n`;
const rust = `// Generated by script/generate-file-icons.mjs from material-icon-theme ${version}
// (${commit}); do not edit by hand. Keys are lowercase and sorted for binary search.

${table("FILE_NAMES", fileNames)}
${table("FILE_EXTENSIONS", fileExtensions)}
${table("FOLDER_NAMES", folderNames)}
pub(super) static ICONS: &[(&str, &[u8])] = &[
${icons
  .map((icon) => `    (${JSON.stringify(icon)}, include_bytes!("../../../assets/icons/material/${icon}.svg")),`)
  .join("\n")}
];
`;
mkdirSync(dirname(generated), { recursive: true });
writeFileSync(generated, rust);
console.log(
  `${icons.length} icons, ${fileNames.size} file names, ${fileExtensions.size} extensions, ${folderNames.size} folder names (material-icon-theme ${version} ${commit})`,
);
