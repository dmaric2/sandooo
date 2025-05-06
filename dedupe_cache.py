#!/usr/bin/env python3
# dedup_cache_csvs.py
"""
Deduplicate the two cache CSVs produced by the indexer **and**
replace their first column ('id') with a fresh 1‥N sequence.

Files handled
-------------
* .cached-pools.csv   – can be 800 MB ➜ streaming dedupe via SQLite on disk
* .cached-tokens.csv  – a few MB ➜ full in-RAM dedupe

Key used for duplicate detection
--------------------------------
Both files: 'address'  (change KEY_COLS_* below if needed).

Python ≥ 3.8 only standard library required.
"""

from __future__ import annotations

import csv
import os
import sqlite3
import tempfile
from pathlib import Path
from typing import List, Sequence

################################################################################
# Small-file, in-RAM variant
################################################################################

def dedupe_small_csv(path: Path, key_cols: Sequence[str]) -> None:
    """Deduplicate the whole CSV in memory, then rewrite with sequential ids."""
    tmp_path = path.with_suffix(path.suffix + ".tmp")

    with path.open(newline="", encoding="utf-8") as f_in, \
         tmp_path.open("w", newline="", encoding="utf-8") as f_out:

        reader = csv.DictReader(f_in)
        fieldnames = reader.fieldnames
        if fieldnames is None:
            raise RuntimeError(f"{path}: missing header")

        writer = csv.DictWriter(f_out, fieldnames=fieldnames)
        writer.writeheader()

        seen: set[tuple[str, ...]] = set()
        next_id = 1
        for row in reader:
            key = tuple(row[c].strip().lower() for c in key_cols)
            if key in seen:
                continue
            seen.add(key)
            row["id"] = str(next_id)
            next_id += 1
            writer.writerow(row)

    os.replace(tmp_path, path)
    print(f"[OK] {path.name}: kept {next_id-1:,} rows with fresh id column")


################################################################################
# Large-file, disk-based variant using SQLite
################################################################################

def dedupe_large_csv(path: Path, key_cols: Sequence[str], batch: int = 50_000) -> None:
    """
    Deduplicate huge CSVs without loading them into RAM.
    After dedupe, rewrite the CSV with a fresh 1‥N id column.
    """
    print(f"[*] {path.name}: scanning …")

    # ------------------------------------------------------------------ build DB
    with tempfile.NamedTemporaryFile(suffix=".sqlite3", delete=False) as db_file:
        db_path = Path(db_file.name)

    conn = sqlite3.connect(db_path)
    cur = conn.cursor()

    with path.open(newline="", encoding="utf-8") as f_in:
        reader = csv.reader(f_in)
        header: List[str] = next(reader)
        col_count = len(header)
        col_names = [f"c{i}" for i in range(col_count)]
        placeholders = ", ".join("?" * col_count)

        # Create table (all TEXT) and UNIQUE index on key columns
        cur.execute(
            f'CREATE TABLE t (rowid INTEGER PRIMARY KEY AUTOINCREMENT, '
            + ", ".join(f"{c} TEXT" for c in col_names) + ");"
        )
        key_expr = ", ".join(f"c{header.index(k)}" for k in key_cols)
        cur.execute(f"CREATE UNIQUE INDEX uniq ON t({key_expr});")

        insert_sql = (
            f'INSERT OR IGNORE INTO t ({", ".join(col_names)}) '
            f"VALUES ({placeholders});"
        )

        # Stream rows in batches
        buf: List[List[str]] = []
        total = 0
        for row in reader:
            total += 1
            buf.append([cell.strip() for cell in row])
            if len(buf) >= batch:
                cur.executemany(insert_sql, buf)
                conn.commit()
                buf.clear()
        if buf:
            cur.executemany(insert_sql, buf)
            conn.commit()

    kept = cur.execute("SELECT COUNT(*) FROM t;").fetchone()[0]
    print(f"[OK] {path.name}: {total:,} rows read → {kept:,} unique")

    # ------------------------------------------------------------------ dump CSV
    tmp_csv = path.with_suffix(path.suffix + ".tmp")
    id_idx = header.index("id")
    with tmp_csv.open("w", newline="", encoding="utf-8") as f_out:
        writer = csv.writer(f_out)
        writer.writerow(header)

        next_id = 1
        for row in cur.execute(
            f'SELECT {", ".join(col_names)} FROM t ORDER BY rowid;'
        ):
            row = list(row)
            row[id_idx] = str(next_id)
            next_id += 1
            writer.writerow(row)

    conn.close()
    db_path.unlink(missing_ok=True)          # delete temp DB
    os.replace(tmp_csv, path)
    print(f"[OK] {path.name}: id column fixed (1‥{next_id-1})")

################################################################################
# Main
################################################################################

def main() -> None:
    base_dir = Path(__file__).resolve().parent / "cache"

    FILES: list[tuple[str, Sequence[str], bool]] = [
        # (filename,               key columns,  large?)
        (".cached-pools.csv",   ["address"],     True),
        (".cached-tokens.csv",  ["address"],     False),
    ]

    for fname, keys, is_large in FILES:
        path = base_dir / fname
        if not path.is_file():
            print(f"[WARN] File not found: {path}")
            continue
        if is_large:
            dedupe_large_csv(path, keys)
        else:
            dedupe_small_csv(path, keys)

    print("All done.")


if __name__ == "__main__":
    main()
