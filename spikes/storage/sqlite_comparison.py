"""Optional M0 comparison using Python's SQLite, never a runtime dependency.

Run: python3 spikes/storage/sqlite_comparison.py
This checks semantics, not relative speed, crash recovery or Rust integration.
"""

import sqlite3
import tempfile
from pathlib import Path


def compare():
    with tempfile.TemporaryDirectory(prefix="kiarni-sqlite-") as directory:
        path = Path(directory) / "db.sqlite"
        with sqlite3.connect(path) as writer:
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("PRAGMA synchronous=FULL")
            writer.execute("""CREATE TABLE documents (
                collection TEXT NOT NULL, id TEXT NOT NULL,
                status TEXT NOT NULL, body TEXT NOT NULL,
                PRIMARY KEY(collection, id))""")
            writer.execute("CREATE INDEX by_status ON documents(collection, status, id)")
            writer.commit()
            row = ("orders", "a", "new", '{"_id":"a","status":"new"}')
            writer.execute("INSERT INTO documents VALUES (?, ?, ?, ?)", row)
            writer.commit()
            try:
                writer.execute("INSERT INTO documents VALUES (?, ?, ?, ?)", row)
            except sqlite3.IntegrityError:
                writer.rollback()
            else:
                raise AssertionError("duplicate ID accepted")
            writer.execute("INSERT INTO documents VALUES (?, ?, ?, ?)",
                           ("orders", "abort", "new", "{}"))
            writer.rollback()
            reader = sqlite3.connect(path)
            try:
                reader.execute("BEGIN")
                assert reader.execute("SELECT status FROM documents").fetchall() == [("new",)]
                writer.execute("UPDATE documents SET status='paid' WHERE id='a'")
                writer.commit()
                assert reader.execute("SELECT status FROM documents").fetchall() == [("new",)]
                reader.rollback()
                assert reader.execute("SELECT status FROM documents").fetchall() == [("paid",)]
            finally:
                reader.close()
            indexed = writer.execute("""SELECT id FROM documents INDEXED BY by_status
                WHERE collection='orders' AND status='paid' ORDER BY id LIMIT 5""").fetchall()
            scanned = writer.execute("""SELECT id FROM documents NOT INDEXED
                WHERE collection='orders' AND status='paid' ORDER BY id LIMIT 5""").fetchall()
            assert indexed == scanned == [("a",)]
            backup_path = Path(directory) / "backup.sqlite"
            with sqlite3.connect(backup_path) as backup:
                writer.backup(backup)
                assert backup.execute("SELECT id, status FROM documents").fetchall() == [("a", "paid")]
        # Connection context managers commit/rollback but require explicit close.
        writer.close()
        backup.close()
        with sqlite3.connect(path) as reopened:
            assert reopened.execute("SELECT id FROM documents").fetchall() == [("a",)]
        reopened.close()
    print(f"SQLite {sqlite3.sqlite_version}: duplicate ID, rollback, WAL snapshot, "
          "index vs scan, online backup and normal reopen passed")


if __name__ == "__main__":
    compare()
