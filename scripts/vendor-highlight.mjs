// Copy highlight.js into src/vendor/highlight/ and index its languages.
//
// The app loads the core and only the grammars a document actually uses, so it
// needs to know, before fetching anything, which file a fence name such as
// `py` or `jl` lives in and which other grammars that one embeds (HTML pulls in
// CSS and JavaScript through `subLanguage`). Both answers come from the
// grammars themselves, so they are computed here once rather than kept by hand.
import { cpSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = resolve(root, "node_modules/@highlightjs/cdn-assets");
const out = resolve(root, "src/vendor/highlight");

rmSync(out, { recursive: true, force: true });
mkdirSync(resolve(out, "languages"), { recursive: true });
cpSync(resolve(pkg, "es/core.min.js"), resolve(out, "core.min.js"));
cpSync(resolve(pkg, "LICENSE"), resolve(out, "LICENSE"));

const { default: hljs } = await import(pathToFileURL(resolve(pkg, "es/core.min.js")));

const names = [];
for (const file of readdirSync(resolve(pkg, "es/languages")).sort()) {
  if (!file.endsWith(".min.js")) continue;
  const name = file.slice(0, -".min.js".length);
  cpSync(resolve(pkg, "es/languages", file), resolve(out, "languages", file));
  const { default: grammar } = await import(
    pathToFileURL(resolve(pkg, "es/languages", file))
  );
  hljs.registerLanguage(name, grammar);
  names.push(name);
}

const aliases = {};
for (const name of names) {
  for (const alias of hljs.getLanguage(name).aliases ?? []) {
    aliases[alias.toLowerCase()] = name;
  }
}
// Fence names the grammars do not claim but Julia documents use: the short
// extension, and Documenter's evaluated blocks, which hold plain Julia source.
for (const alias of ["jl", "@example", "@repl", "@setup", "@eval"]) {
  aliases[alias] ??= "julia";
}
// A canonical name always wins over another grammar's alias of the same name.
for (const name of names) aliases[name] = name;

/// Every grammar a language embeds, by walking its mode tree for `subLanguage`.
function embedded(language) {
  const found = new Set();
  const seen = new Set();
  const walk = (mode) => {
    if (!mode || typeof mode !== "object" || seen.has(mode)) return;
    seen.add(mode);
    for (const sub of [mode.subLanguage ?? []].flat()) {
      const target = aliases[sub.toLowerCase()];
      if (target) found.add(target);
    }
    for (const key of ["contains", "variants", "starts"]) {
      for (const child of [mode[key] ?? []].flat()) walk(child);
    }
  };
  walk(language);
  return found;
}

const requires = {};
for (const name of names) {
  const deps = [...embedded(hljs.getLanguage(name))].filter((dep) => dep !== name);
  if (deps.length) requires[name] = deps.sort();
}

writeFileSync(resolve(out, "index.json"), `${JSON.stringify({ aliases, requires })}\n`);
console.log(`highlight.js: vendored ${names.length} languages`);
