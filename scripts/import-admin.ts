// One-time import of the workbench's offline administrative extent catalog.
import { Database } from "bun:sqlite";
import { mkdir } from "node:fs/promises";
const source = process.argv[2];
if (!source) throw new Error("Usage: bun scripts/import-admin.ts <administrative_boundaries.db>");
const db = new Database(source, { readonly: true });
const rows = db.query("SELECT name,code,level,south,north,west,east FROM boundaries ORDER BY code").all();
await mkdir("src-tauri/resources", { recursive: true });
await Bun.write("src-tauri/resources/admin-regions.json", JSON.stringify(rows));
console.log(JSON.stringify({ count: rows.length, sample: rows.slice(0, 5) }));
db.close();
