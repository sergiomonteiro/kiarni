"""Check workspace path dependencies from apps/crates to experimental packages."""

import json
from pathlib import Path
import subprocess
import sys


def main():
    root = Path(__file__).resolve().parents[1]
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked", "--offline"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    packages = json.loads(result.stdout)["packages"]
    by_path = {Path(package["manifest_path"]).parent: package for package in packages}
    product = [
        path for path in by_path
        if path.is_relative_to(root / "apps") or path.is_relative_to(root / "crates")
    ]
    violations = []
    for origin in product:
        pending = [(origin, [by_path[origin]["name"]])]
        visited = set()
        while pending:
            path, chain = pending.pop()
            if path in visited:
                continue
            visited.add(path)
            if path.is_relative_to(root / "spikes"):
                violations.append(" -> ".join(chain))
                continue
            for dependency in by_path[path]["dependencies"]:
                target = dependency.get("path")
                if target and dependency["kind"] != "dev":
                    target = Path(target)
                    if target in by_path:
                        pending.append((target, chain + [by_path[target]["name"]]))
    if violations:
        print("Product dependencies reach spikes:", file=sys.stderr)
        print("\n".join(violations), file=sys.stderr)
        return 1
    print(f"Product dependency boundary passed for {len(product)} packages.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
