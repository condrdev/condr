#!/usr/bin/env node
// Vendor JetBrains icons and filename associations at a fixed upstream revision.
// Run: node script/generate-file-icons.mjs
// Native file types override bundled TextMate associations. PSI, content detection
// and project-root markers require an IDE project model; Condr has filesystem data.

import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const revision = "50461b71767a52e286026538af67b05552d75f6e";
const upstream = `https://raw.githubusercontent.com/JetBrains/intellij-community/${revision}/`;
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const assetsDir = join(repo, "crates/condr-gui/assets/icons/jetbrains");
const generated = join(repo, "crates/condr-gui/src/app/file_icons/generated.rs");
const requests = new Map();
function source(path, optional = false) {
  if (!requests.has(path)) {
    requests.set(path, fetch(upstream + path).then(async (response) => {
      if (optional && response.status === 404) return null;
      if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
      return response.text();
    }));
  }
  return requests.get(path);
}

// Use the New UI resource (expui) wherever provided by the platform or plugin.
const icons = {
  unknown: "platform/icons/src/expui/fileTypes/unknown",
  folder: "platform/icons/src/expui/nodes/folder",
  text: "platform/icons/src/expui/fileTypes/text",
  archive: "platform/icons/src/expui/fileTypes/archive",
  image: "platform/icons/src/expui/fileTypes/image",
  java: "platform/icons/src/expui/fileTypes/java",
  javaClass: "platform/icons/src/expui/fileTypes/javaClass",
  c: "platform/icons/src/expui/fileTypes/c",
  cpp: "platform/icons/src/expui/fileTypes/cpp",
  csharp: "platform/icons/src/expui/fileTypes/Csharp",
  css: "platform/icons/src/expui/fileTypes/css",
  dockerfile: "platform/icons/src/expui/fileTypes/docker",
  gitignore: "platform/icons/src/expui/fileTypes/gitignore",
  editorconfig: "platform/icons/src/expui/fileTypes/editorConfig",
  html: "platform/icons/src/expui/fileTypes/html",
  xhtml: "platform/icons/src/expui/fileTypes/xhtml",
  javascript: "platform/icons/src/expui/fileTypes/javaScript",
  typescript: "platform/icons/src/expui/fileTypes/typeScript",
  json: "platform/icons/src/expui/fileTypes/json",
  markdown: "platform/icons/src/expui/fileTypes/markdown",
  properties: "platform/icons/src/expui/fileTypes/properties",
  shellscript: "platform/icons/src/expui/fileTypes/text", // ShFileType: AllIcons.Nodes.Console
  sql: "platform/icons/src/expui/fileTypes/sql",
  swift: "platform/icons/src/expui/fileTypes/swiftLang",
  toml: "platform/icons/src/expui/fileTypes/toml",
  xml: "platform/icons/src/expui/fileTypes/xml",
  yaml: "platform/icons/src/expui/fileTypes/yaml",
  diff: "platform/icons/src/expui/fileTypes/patch",
  bat: "platform/icons/src/expui/fileTypes/microsoftWindows",
  groovy: "platform/icons/src/expui/fileTypes/groovy",
  perl: "platform/icons/src/expui/fileTypes/perl",
  restructuredtext: "platform/icons/src/expui/fileTypes/rst",
  go: "platform/icons/src/language/go",
  rust: "platform/icons/src/language/rust",
  ruby: "platform/icons/src/language/ruby",
  php: "platform/icons/src/expui/language/php",
  kotlin: "plugins/kotlin/base/resources/resources/org/jetbrains/kotlin/idea/icons/expui/kotlin",
  python: "python/python-parser/resources/icons/com/jetbrains/python/parser/expui/python",
};

const names = new Map();
const namesInsensitive = new Map();
const extensions = new Map();
const patterns = new Map();
const aliases = { javascriptreact: "javascript", typescriptreact: "typescript", jsonc: "json", jsonl: "json", "c++": "cpp" };
const bundles = ["bat", "cpp", "csharp", "css", "diff", "docker", "go", "groovy", "html", "java", "javascript", "json", "kotlin", "markdown-basics", "perl", "php", "python", "restructuredtext", "ruby", "rust", "shellscript", "sql", "swift", "typescript-basics", "xml", "yaml"];
for (const bundle of bundles) {
  const data = JSON.parse(await source(`plugins/textmate/lib/bundles/${bundle}/package.json`));
  for (const language of data.contributes.languages ?? []) {
    const icon = aliases[language.id] ?? language.id;
    if (!icons[icon]) continue;
    for (const ext of language.extensions ?? []) extensions.set(ext.slice(1).toLowerCase(), icon);
    // TextMateServiceImpl normalizes bundle names to lowercase before lookup.
    for (const name of language.filenames ?? []) namesInsensitive.set(name.toLowerCase(), icon);
    // Path-qualified grammar associations need more than a directory entry's name.
    for (const pattern of language.filenamePatterns ?? []) {
      if (!pattern.includes("/")) patterns.set(pattern, icon);
    }
  }
}

// Import native associations rather than maintaining another copy of their names.
const registrations = [
  ["platform/platform-impl/resources/intellij.platform.ide.impl.xml", { ARCHIVE: "archive", PLAIN_TEXT: "text" }],
  ["platform/vcs-impl/resources/META-INF/VcsExtensions.xml", { PATCH: "diff" }],
  ["json/resources/intellij.json.xml", { JSON: "json", JSON5: "json", "JSON-lines": "json" }],
  ["xml/xml-psi-impl/resources/intellij.xml.psi.impl.xml", { HTML: "html", XHTML: "xhtml", XML: "xml", DTD: "xml" }],
  ["python/python-parser/resources/intellij.python.parser.xml", { Python: "python", PythonStub: "python" }],
  ["plugins/toml/core/src/main/resources/intellij.toml.core.xml", { TOML: "toml" }],
  ["plugins/markdown/core/resources/META-INF/plugin.xml", { Markdown: "markdown" }],
  ["plugins/sh/core/resources/intellij.sh.core.xml", { "Shell Script": "shellscript" }],
  ["plugins/yaml/resources/intellij.yaml.xml", { YAML: "yaml" }],
  ["plugins/properties/properties-common/resources/intellij.properties.xml", { Properties: "properties" }],
  ["plugins/editorconfig/common/resources/intellij.editorconfig.common.xml", { EditorConfig: "editorconfig" }],
  ["plugins/git4idea/backend/resources/intellij.vcs.git.backend.xml", { "GitIgnore file": "gitignore" }],
];
for (const [path, types] of registrations) {
  const xml = await source(path);
  for (const [, declaration] of xml.matchAll(/<fileType\s+([^>]+)>/g)) {
    const attributes = Object.fromEntries([...declaration.matchAll(/(\w+)="([^"]*)"/g)].map(([, key, value]) => [key, value]));
    const icon = types[attributes.name];
    if (!icon) continue;
    for (const [attribute, table, insensitive] of [
      ["extensions", extensions, true], ["fileNames", names, false],
      ["fileNamesCaseInsensitive", namesInsensitive, true], ["patterns", patterns, false],
    ]) {
      for (const value of attributes[attribute]?.split(";") ?? []) table.set(insensitive ? value.toLowerCase() : value, icon);
    }
  }
}

extensions.set("class", "javaClass");
for (const ext of ["png", "bmp", "gif", "ico", "jpg", "jpeg", "tif", "tiff", "webp", "svg"]) extensions.set(ext, "image");
// These upstream patterns need at most one '*'; fail if that assumption changes.
for (const pattern of patterns.keys()) {
  if (pattern.includes("?") || pattern.split("*").length !== 2) throw new Error(`Unsupported filename pattern: ${pattern}`);
}

// Finish all downloads before replacing the committed assets.
const assets = new Map();
for (const [icon, path] of Object.entries(icons)) {
  const light = await source(`${path}.svg`);
  const dark = await source(`${path}_dark.svg`, true);
  assets.set(icon, light);
  assets.set(`${icon}_dark`, dark ?? light);
}
mkdirSync(assetsDir, { recursive: true });
for (const name of readdirSync(assetsDir)) {
  if (name.endsWith(".svg")) rmSync(join(assetsDir, name));
}
for (const [icon, svg] of assets) writeFileSync(join(assetsDir, `${icon}.svg`), svg);

const table = (name, entries) => `pub(super) static ${name}: &[(&str, &str)] = &[\n${entries.map(([key, icon]) => `    (${JSON.stringify(key)}, ${JSON.stringify(icon)}),`).join("\n")}\n];\n`;
const sorted = (map) => [...map].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
const rust = `// Generated by script/generate-file-icons.mjs from JetBrains/intellij-community
// ${revision}; do not edit by hand.
// Upstream Apache-2.0 and MIT notices: assets/icons/NOTICE-FILE-ICONS.

${table("FILE_NAMES", sorted(names))}
${table("FILE_NAMES_INSENSITIVE", sorted(namesInsensitive))}
${table("FILE_EXTENSIONS", sorted(extensions))}
${table("FILE_PATTERNS", [...patterns].sort(([a], [b]) => b.length - a.length || (a < b ? -1 : a > b ? 1 : 0)))}
pub(super) static ICONS: &[(&str, &[u8])] = &[
${sorted(assets).map(([icon]) => `    (${JSON.stringify(icon)}, include_bytes!("../../../assets/icons/jetbrains/${icon}.svg")),`).join("\n")}
];
`;
writeFileSync(generated, rust);
execFileSync("rustfmt", ["--edition", "2024", generated]);
console.log(`${assets.size} SVGs, ${names.size + namesInsensitive.size} names, ${extensions.size} extensions, ${patterns.size} patterns (${revision})`);
