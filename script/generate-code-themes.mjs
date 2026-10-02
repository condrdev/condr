#!/usr/bin/env node
// Vendor the Preview and Diff Tabs' code themes that two-face does not bundle, at fixed
// upstream revisions, as .tmTheme files syntect loads (ADR 0032). Tokyo Night ships
// tmThemes and is copied unchanged; GitHub's VS Code themes and JetBrains' editor
// schemes are converted. Run with Node.js after changing a revision or a mapping.
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(repo, "crates/condr-gui/assets/code_themes");

const upstream = {
  tokyonight: {
    url: "https://github.com/folke/tokyonight.nvim",
    revision: "cdc07ac78467a233fd62c493de29a17e0cf2b2b6",
  },
  // Shiki's copy of primer/github-vscode-theme's released themes.
  github: {
    url: "https://github.com/shikijs/textmate-grammars-themes",
    revision: "37edd1b26f18838050661d912334aba0ca7f4931",
  },
  jetbrains: {
    url: "https://github.com/JetBrains/intellij-community",
    revision: "be6fb1281d6d55af955e3db68af81b8a6f9a6d4c",
  },
};

async function get(source, path) {
  const { url, revision } = upstream[source];
  const raw = url.replace("https://github.com/", "https://raw.githubusercontent.com/");
  const response = await fetch(`${raw}/${revision}/${path}`);
  if (!response.ok) throw new Error(`${source} ${path}: HTTP ${response.status}`);
  return response.text();
}

// A tmTheme plist: global settings first, then one dict per scope rule.
function tmTheme(name, global, rules) {
  const escape = (text) =>
    text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const dict = (entries, indent) =>
    [
      `${indent}<dict>`,
      ...Object.entries(entries)
        .filter(([, value]) => value)
        .flatMap(([key, value]) =>
          typeof value === "object"
            ? [`${indent}\t<key>${key}</key>`, dict(value, `${indent}\t`)]
            : [`${indent}\t<key>${key}</key>`, `${indent}\t<string>${escape(value)}</string>`],
        ),
      `${indent}</dict>`,
    ].join("\n");
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">',
    "<dict>",
    "\t<key>name</key>",
    `\t<string>${escape(name)}</string>`,
    "\t<key>settings</key>",
    "\t<array>",
    dict({ settings: global }, "\t\t"),
    ...rules.map((rule) => dict(rule, "\t\t")),
    "\t</array>",
    "</dict>",
    "</plist>",
    "",
  ].join("\n");
}

// A VS Code color theme: editor colours become the global settings, `tokenColors` the rules.
function fromVsCode(name, json) {
  const theme = JSON.parse(json);
  const colors = theme.colors ?? {};
  const global = {
    background: colors["editor.background"],
    foreground: colors["editor.foreground"],
    caret: colors["editorCursor.foreground"],
    lineHighlight: colors["editor.lineHighlightBackground"],
    selection: colors["editor.selectionBackground"],
    gutterForeground: colors["editorLineNumber.foreground"],
  };
  const rules = (theme.tokenColors ?? [])
    .filter((rule) => rule.scope && rule.settings)
    .map((rule) => ({
      name: rule.name,
      scope: [rule.scope].flat().join(", "),
      settings: {
        foreground: rule.settings.foreground,
        background: rule.settings.background,
        fontStyle: rule.settings.fontStyle,
      },
    }));
  return tmTheme(name, global, rules);
}

// JetBrains editor schemes are XML: <scheme parent_scheme> with <colors> and <attributes>.
function parseXml(text) {
  const root = { children: [] };
  const stack = [root];
  const tags = /<(\/?)([\w:-]+)((?:\s+[\w:-]+="[^"]*")*)\s*(\/?)>/g;
  for (const [, close, name, raw, selfClosing] of text.replace(/<!--[\s\S]*?-->/g, "").matchAll(tags)) {
    if (close) {
      stack.pop();
      continue;
    }
    const attrs = Object.fromEntries([...raw.matchAll(/([\w:-]+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
    const node = { name, attrs, children: [] };
    stack.at(-1).children.push(node);
    if (!selfClosing) stack.push(node);
  }
  return root;
}

function schemes(...documents) {
  const found = {};
  for (const document of documents) {
    const walk = (node) => {
      if (node.name === "scheme") {
        const section = (name) => node.children.find((child) => child.name === name)?.children ?? [];
        found[node.attrs.name] = {
          parent: node.attrs.parent_scheme,
          colors: Object.fromEntries(section("colors").map((option) => [option.attrs.name, option.attrs.value])),
          attributes: Object.fromEntries(section("attributes").map((option) => [option.attrs.name, option])),
        };
      }
      node.children.forEach(walk);
    };
    walk(parseXml(document));
  }
  return found;
}

// The nearest scheme that sets `key` wins; an option with only `baseAttributes` inherits.
function attribute(all, scheme, key) {
  for (let current = all[scheme]; current; current = all[current.parent]) {
    const option = current.attributes[key];
    if (!option) continue;
    const value = option.children.find((child) => child.name === "value");
    if (!value && option.attrs.baseAttributes) return attribute(all, scheme, option.attrs.baseAttributes);
    return Object.fromEntries((value?.children ?? []).map((field) => [field.attrs.name, field.attrs.value]));
  }
  return {};
}

function color(all, scheme, key) {
  for (let current = all[scheme]; current; current = all[current.parent]) {
    if (current.colors[key]) return hex(current.colors[key]);
  }
}

// JetBrains writes RGB without leading zeros: `33b3` is #0033b3.
const hex = (value) => (value ? `#${value.padStart(6, "0")}` : undefined);
const fontStyles = { 1: "bold", 2: "italic", 3: "bold italic" };

// Which TextMate scopes each JetBrains attribute colours. TextMate picks the most
// specific selector, so `constant` here does not override `constant.numeric`.
const jetbrainsScopes = [
  ["DEFAULT_LINE_COMMENT", "comment"],
  ["DEFAULT_BLOCK_COMMENT", "comment.block"],
  ["DEFAULT_DOC_COMMENT", "comment.block.documentation, comment.line.documentation"],
  ["DEFAULT_KEYWORD", "keyword, storage, constant.language, variable.language, markup.heading, entity.name.section"],
  ["DEFAULT_OPERATION_SIGN", "keyword.operator"],
  ["DEFAULT_STRING", "string, markup.raw"],
  ["DEFAULT_VALID_STRING_ESCAPE", "constant.character.escape"],
  ["DEFAULT_NUMBER", "constant.numeric"],
  ["DEFAULT_CONSTANT", "constant, variable.other.constant, entity.name.constant"],
  ["DEFAULT_FUNCTION_DECLARATION", "entity.name.function"],
  ["DEFAULT_FUNCTION_CALL", "variable.function, support.function"],
  ["DEFAULT_CLASS_NAME", "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.other.inherited-class, support.type, support.class"],
  ["DEFAULT_INTERFACE_NAME", "entity.name.interface, entity.name.trait"],
  ["DEFAULT_INSTANCE_FIELD", "variable.other.member, variable.other.property, support.type.property-name"],
  ["DEFAULT_PARAMETER", "variable.parameter"],
  ["DEFAULT_METADATA", "meta.annotation, storage.type.annotation, entity.name.function.decorator"],
  ["DEFAULT_PREDEFINED_SYMBOL", "support.function.builtin, support.type.builtin"],
  ["DEFAULT_TAG", "entity.name.tag"],
  ["DEFAULT_ATTRIBUTE", "entity.other.attribute-name"],
  ["DEFAULT_ENTITY", "constant.character.entity"],
  ["DEFAULT_LABEL", "entity.name.label"],
];

function fromJetBrains(name, all, scheme) {
  const text = attribute(all, scheme, "TEXT");
  const global = {
    background: hex(text.BACKGROUND) ?? "#ffffff",
    foreground: hex(text.FOREGROUND) ?? "#000000",
    caret: color(all, scheme, "CARET_COLOR"),
    lineHighlight: color(all, scheme, "CARET_ROW_COLOR"),
    selection: color(all, scheme, "SELECTION_BACKGROUND"),
    gutterForeground: color(all, scheme, "LINE_NUMBERS_COLOR"),
  };
  const rules = jetbrainsScopes.flatMap(([key, scope]) => {
    const value = attribute(all, scheme, key);
    const settings = {
      foreground: hex(value.FOREGROUND),
      background: hex(value.BACKGROUND),
      fontStyle: fontStyles[value.FONT_TYPE],
    };
    return Object.values(settings).some(Boolean) ? [{ name: key, scope, settings }] : [];
  });
  // Diffs take the IDE's own file-status colours.
  for (const [scope, key] of [
    ["markup.inserted", "FILESTATUS_ADDED"],
    ["markup.deleted", "FILESTATUS_DELETED"],
    ["meta.diff.range, meta.diff.header", "FILESTATUS_MODIFIED"],
  ]) {
    rules.push({ name: key, scope, settings: { foreground: color(all, scheme, key) } });
  }
  return tmTheme(name, global, rules);
}

const [tokyoDay, tokyoNight, githubLight, githubDark, jetbrainsDefaults, jetbrainsLight] =
  await Promise.all([
    get("tokyonight", "extras/sublime/tokyonight_day.tmTheme"),
    get("tokyonight", "extras/sublime/tokyonight_night.tmTheme"),
    get("github", "packages/tm-themes/themes/github-light-default.json"),
    get("github", "packages/tm-themes/themes/github-dark-default.json"),
    get("jetbrains", "platform/platform-resources/src/DefaultColorSchemesManager.xml"),
    get("jetbrains", "platform/platform-resources/src/themes/Light.xml"),
  ]);
const jetbrains = schemes(jetbrainsDefaults, jetbrainsLight);

const themes = {
  "tokyo-night-day.tmTheme": tokyoDay,
  "tokyo-night.tmTheme": tokyoNight,
  "github-light.tmTheme": fromVsCode("GitHub Light", githubLight),
  "github-dark.tmTheme": fromVsCode("GitHub Dark", githubDark),
  "jetbrains-light.tmTheme": fromJetBrains("JetBrains Light", jetbrains, "IntelliJ Light"),
  "jetbrains-darcula.tmTheme": fromJetBrains("JetBrains Darcula", jetbrains, "Darcula"),
};

await mkdir(outDir, { recursive: true });
for (const [file, contents] of Object.entries(themes)) {
  await writeFile(join(outDir, file), contents);
}
const revisions = Object.entries(upstream)
  .map(([name, { url, revision }]) => `  ${name}: ${url} at ${revision}`)
  .join("\n");
await writeFile(
  join(outDir, "NOTICE"),
  `The code themes in this directory are generated by script/generate-code-themes.mjs
from these upstream revisions:
${revisions}

tokyo-night-day.tmTheme and tokyo-night.tmTheme are extras/sublime/tokyonight_day and
tokyonight_night from folke/tokyonight.nvim, unchanged. They are licensed under
Apache-2.0; the complete Apache License 2.0 is in the repository's LICENSE and the
bundled Settings > Licenses text.

github-light.tmTheme and github-dark.tmTheme are converted from the GitHub Light
Default and GitHub Dark Default themes of primer/github-vscode-theme, as Shiki
redistributes them. They are licensed under the MIT License:

MIT License

Copyright (c) 2020 Primer

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

jetbrains-light.tmTheme and jetbrains-darcula.tmTheme take their colours from the
IntelliJ Light and Darcula editor schemes of JetBrains/intellij-community
(platform/platform-resources/src/themes/Light.xml and DefaultColorSchemesManager.xml),
mapped onto TextMate scopes by the script. They are licensed under Apache-2.0; the
complete Apache License 2.0 is in the repository's LICENSE and the bundled
Settings > Licenses text. Upstream NOTICE:

This software includes code from IntelliJ IDEA
Copyright (C) JetBrains s.r.o.
https://www.jetbrains.com/idea/
`,
);
console.log(`wrote ${Object.keys(themes).length} themes to ${outDir}`);
