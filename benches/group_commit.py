"""Record the isolated preformed group experiment and its environment."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from http_load import ROOT, environment, filesystem


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--collector", action="store_true")
    mode.add_argument("--open-loop", action="store_true")
    parser.add_argument("--seconds", type=int)
    parser.add_argument("--repetitions", type=int, default=3)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output must be a new file")
    seconds = args.seconds if args.seconds is not None else (10 if args.open_loop else 2)
    os.chdir(ROOT)
    name = "collector_open_loop" if args.open_loop else ("collector_bench" if args.collector else "kiarni-group-commit-spike")
    binary = ROOT / "target/release" / name
    metadata = environment(binary)
    digest = hashlib.sha256()
    for path in sorted((ROOT / "spikes/group-commit").rglob("*")):
        if path.is_file():
            digest.update(str(path.relative_to(ROOT)).encode())
            digest.update(path.read_bytes())
    metadata["spike_source_sha256"] = digest.hexdigest()
    metadata["runner_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    metadata["data_filesystem"] = filesystem(Path(tempfile.gettempdir()).resolve())
    command = [str(binary)]
    if args.collector or args.open_loop:
        command.extend(["--seconds", str(seconds), "--repetitions", str(args.repetitions)])
    metadata["command"] = command
    measured = subprocess.run(command, check=True, stdout=subprocess.PIPE, text=True)
    report = {"schema": 1, "environment": metadata, "measurement": json.loads(measured.stdout)}
    descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    for trial in report["measurement"]["trials"]:
        if args.open_loop:
            print(f"rate={trial['target_rate']} window={trial['window_us']} "
                  f"confirmed/s={trial['confirmed_per_second']:.2f} full={trial['full']} "
                  f"skipped={trial['generator_skipped']}")
            continue
        if args.collector:
            print(f"clients={trial['clients']} window={trial['window_us']} "
                  f"confirmed/s={trial['confirmed_per_second']:.2f} full={trial['full']} "
                  f"p95={trial['latency']['p95_ms']:.3f}ms")
            continue
        print(f"group={trial['group_size']} rep={trial['repetition']} "
              f"confirmed/s={trial['confirmed_operations_per_second']:.2f} "
              f"group p95={trial['group_p95_ms']:.3f}ms")


if __name__ == "__main__":
    main()
