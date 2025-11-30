import argparse
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--samply", action="store_true")
args = parser.parse_args()

proj_dir = Path(__file__).parent.resolve()
subprocess.check_call(["cargo", "build", "--release", "--quiet"], cwd=proj_dir)
subprocess.check_call(
    [
        *(["samply", "record"] if args.samply else ["time"]),
        proj_dir / "target" / "release" / "frenzy",
    ],
    cwd=proj_dir,
)
