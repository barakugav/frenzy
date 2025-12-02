import argparse
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--samply", action="store_true")
args = parser.parse_args()

proj_dir = Path(__file__).parent.resolve()

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
    [*exe, proj_dir / "target" / profile / "frenzy"],
    cwd=proj_dir,
)
