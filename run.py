import argparse
import subprocess
from pathlib import Path

proj_dir = Path(__file__).parent.resolve()


parser = argparse.ArgumentParser()
parser.add_argument("--samply", action="store_true")
parser.add_argument(
    "--measurements-file", type=Path, default=proj_dir / "1brc" / "measurements_1b.txt"
)
args = parser.parse_args()


if args.samply:
    exe = ["samply", "record"]
    profile = "profiling"
else:
    exe = ["time"]
    profile = "release"

subprocess.check_call(
    ["cargo", "build", f"--profile={profile}", "--quiet"], cwd=proj_dir
)
subprocess.check_call(
    [*exe, proj_dir / "target" / profile / "frenzy", args.measurements_file],
)
