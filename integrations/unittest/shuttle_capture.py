"""Bound the upstream capture artifacts into one executor-owned stdout receipt."""
import contextlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import capture_unittest

if __name__ == "__main__":
    # The upstream helper owns runner callbacks and normalization. Keep arbitrary
    # test output on the executor's independently bounded stderr stream.
    with contextlib.redirect_stdout(sys.stderr):
        code = capture_unittest.main()
    output = Path(sys.argv[sys.argv.index("--output") + 1])
    raw = output.with_suffix(".raw.json")
    if output.stat().st_size > 65536 or raw.stat().st_size > 65536:
        raise SystemExit("capture exceeds bound")
    print(json.dumps({"bundle": json.loads(output.read_text(encoding="utf-8")),
                      "raw": raw.read_text(encoding="utf-8")}))
    raise SystemExit(code)
