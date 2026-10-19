"""Launch the independent agentbus crate from any working directory."""

import os
from pathlib import Path
import subprocess
import sys


def main(args=None):
    arguments = sys.argv[1:] if args is None else args
    if os.environ.get("AGENTBUS_RELAY_DIR") or (
        os.environ.get("SANDBOX_NAME") and not os.environ.get("AGENTBUS_DB")
    ):
        # The relay owns SQLite on the host; sandbox filesystems need not
        # implement SQLite's locking or cross-process shared memory.
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        from relay import client_main, relay_directory

        directory = relay_directory(Path(__file__).resolve().parents[2])
        if directory is not None:
            return client_main(directory, arguments)
    environment = os.environ.copy()
    # The compiler's cross-language LTO flags do not apply to agentbus.
    # An empty encoded value also overrides flags in ancestor Cargo configs.
    environment["RUSTFLAGS"] = ""
    environment["CARGO_ENCODED_RUSTFLAGS"] = ""
    manifest = Path(__file__).resolve().with_name("Cargo.toml")
    return subprocess.run(
        [
            "cargo", "run", "--quiet", "--manifest-path", str(manifest), "--",
            *arguments,
        ],
        env=environment,
    ).returncode


if __name__ == "__main__":
    sys.exit(main())
