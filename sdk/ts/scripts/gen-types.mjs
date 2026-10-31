// Generates src/types.gen.ts from the API schema the protocol crate publishes
// (schemas/garage-plugin-api.schema.json, written by
// `UPDATE_SCHEMA=1 cargo test -p garage-protocol`).
// `--check` compares instead of writing, so a stale file fails `npm test`.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { compile } from "json-schema-to-typescript";

const here = dirname(fileURLToPath(import.meta.url));
const schemaPath = join(here, "../../../schemas/garage-plugin-api.schema.json");
const outPath = join(here, "../src/types.gen.ts");

const schema = JSON.parse(readFileSync(schemaPath, "utf8"));
const out = await compile(schema, "ApiTypes", {
  additionalProperties: false,
  bannerComment:
    "/* Generated from schemas/garage-plugin-api.schema.json by scripts/gen-types.mjs.\n * Do not edit. Run `npm run gen` after the schema changes. */",
  format: true,
  style: { singleQuote: false },
});

if (process.argv.includes("--check")) {
  let current = "";
  try {
    current = readFileSync(outPath, "utf8");
  } catch {}
  if (current !== out) {
    console.error("src/types.gen.ts is stale; run `npm run gen` and commit it.");
    process.exit(1);
  }
  console.log("src/types.gen.ts is up to date.");
} else {
  writeFileSync(outPath, out);
  console.log("wrote src/types.gen.ts");
}
