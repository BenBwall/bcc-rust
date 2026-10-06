"""Launch the independent agentbus crate from any working directory."""

import os
from pathlib import Path
import subprocess
import sys


def main(args=None):
    environment = os.environ.copy()
    # The compiler's cross-language LTO flags do not apply to agentbus.
    # An empty encoded value also overrides flags in ancestor Cargo configs.
    environment["RUSTFLAGS"] = ""
    environment["CARGO_ENCODED_RUSTFLAGS"] = ""
    manifest = Path(__file__).resolve().with_name("Cargo.toml")
    return subprocess.run(
        [
            "cargo", "run", "--quiet", "--manifest-path", str(manifest), "--",
            *(sys.argv[1:] if args is None else args),
        ],
        env=environment,
    ).returncode


if __name__ == "__main__":
    sys.exit(main())
