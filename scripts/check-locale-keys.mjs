// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

// Reject duplicate keys in locale JSON. RFC 8259 makes object-key uniqueness a
// SHOULD, not a MUST, so `JSON.parse` silently keeps the LAST value for a
// repeated key — e.g. `setup.json` once had `clone` as both a leaf string and a
// nested object, and the string was silently dropped. That breaks only at
// render time (and only surfaces as a dev-mode intlify warning), so it slips
// past `JSON.parse`, prettier, and vue-tsc. This walks the raw text with a tiny
// parser that throws on a repeat, with the key path and position for a fix.
//
// Zero deps (node:fs/node:path only) so it runs in CI without an install.

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

class ParseError extends Error {}

/** Parse JSON, throwing ParseError on duplicate keys or any syntax error. */
function parseRejectingDupes(text) {
  let i = 0;
  const n = text.length;
  const path = []; // breadcrumb of enclosing object keys / array indices

  const loc = () => {
    let line = 1;
    let col = 1;
    for (let j = 0; j < i && j < n; j++) {
      if (text[j] === "\n") {
        line++;
        col = 1;
      } else col++;
    }
    return `${line}:${col}`;
  };
  const fail = (msg) => new ParseError(`${msg} (at ${loc()})`);

  const skipWs = () => {
    while (i < n) {
      const c = text[i];
      if (c === " " || c === "\t" || c === "\n" || c === "\r") i++;
      else break;
    }
  };

  function parseValue() {
    skipWs();
    const c = text[i];
    if (c === "{") return parseObject();
    if (c === "[") return parseArray();
    if (c === '"') return parseString();
    if (c === "-" || (c >= "0" && c <= "9")) return parseNumber();
    if (text.startsWith("true", i)) return ((i += 4), true);
    if (text.startsWith("false", i)) return ((i += 5), false);
    if (text.startsWith("null", i)) return ((i += 4), null);
    throw fail(`Unexpected character '${c ?? "<eof>"}'`);
  }

  function parseObject() {
    const obj = Object.create(null);
    const seen = new Set();
    i++; // consume '{'
    skipWs();
    if (text[i] === "}") {
      i++;
      return obj;
    }
    for (;;) {
      skipWs();
      const key = parseString();
      path.push(key);
      if (seen.has(key)) {
        const where = path.slice(0, -1).join(".") || "<root>";
        throw fail(`Duplicate key "${key}" under "${where}"`);
      }
      seen.add(key);
      skipWs();
      if (text[i] !== ":") throw fail("Expected ':' after key");
      i++;
      obj[key] = parseValue();
      path.pop();
      skipWs();
      if (text[i] === ",") {
        i++;
        continue;
      }
      if (text[i] === "}") {
        i++;
        return obj;
      }
      throw fail("Expected ',' or '}'");
    }
  }

  function parseArray() {
    const arr = [];
    let idx = 0;
    i++; // consume '['
    skipWs();
    if (text[i] === "]") {
      i++;
      return arr;
    }
    for (;;) {
      path.push(idx++);
      const value = parseValue();
      path.pop();
      arr.push(value);
      skipWs();
      if (text[i] === ",") {
        i++;
        continue;
      }
      if (text[i] === "]") {
        i++;
        return arr;
      }
      throw fail("Expected ',' or ']'");
    }
  }

  function parseString() {
    if (text[i] !== '"') throw fail("Expected '\"'");
    i++;
    let out = "";
    while (i < n) {
      const c = text[i];
      if (c === '"') {
        i++;
        return out;
      }
      if (c === "\\") {
        i++;
        const e = text[i];
        const simple = {
          '"': '"',
          "\\": "\\",
          "/": "/",
          b: "\b",
          f: "\f",
          n: "\n",
          r: "\r",
          t: "\t",
        };
        if (e in simple) {
          out += simple[e];
        } else if (e === "u") {
          const hex = text.slice(i + 1, i + 5);
          if (!/^[0-9a-fA-F]{4}$/.test(hex)) throw fail("Invalid \\u escape");
          out += String.fromCharCode(parseInt(hex, 16));
          i += 4;
        } else {
          throw fail(`Invalid escape '\\${e}'`);
        }
        i++;
      } else {
        out += c;
        i++;
      }
    }
    throw fail("Unterminated string");
  }

  function parseNumber() {
    const start = i;
    if (text[i] === "-") i++;
    while (i < n && text[i] >= "0" && text[i] <= "9") i++;
    if (text[i] === ".") {
      i++;
      while (i < n && text[i] >= "0" && text[i] <= "9") i++;
    }
    if (text[i] === "e" || text[i] === "E") {
      i++;
      if (text[i] === "+" || text[i] === "-") i++;
      while (i < n && text[i] >= "0" && text[i] <= "9") i++;
    }
    return Number(text.slice(start, i));
  }

  const value = parseValue();
  skipWs();
  if (i !== n) throw fail("Trailing content after JSON value");
  return value;
}

function* walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(p);
    else if (entry.name.endsWith(".json")) yield p;
  }
}

const localesDir = fileURLToPath(new URL("../src/locales", import.meta.url));
const files = [...walk(localesDir)].sort();

let failed = 0;
for (const file of files) {
  try {
    parseRejectingDupes(readFileSync(file, "utf8"));
  } catch (e) {
    console.error(`${file}: ${e.message}`);
    failed++;
  }
}

if (failed) {
  console.error(
    `\n${failed}/${files.length} locale file(s) have duplicate or invalid JSON.`,
  );
  process.exit(1);
}
console.log(`OK: no duplicate keys across ${files.length} locale file(s).`);
