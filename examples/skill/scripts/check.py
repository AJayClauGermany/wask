"""Checks a release-notes draft. The `file` argument arrives as $file."""
import os
import sys

path = os.environ["file"]
long_lines = [n for n, line in enumerate(open(path), 1) if len(line.rstrip("\n")) > 100]
for n in long_lines:
    print(f"{path}:{n}: line longer than 100 characters")
if os.environ.get("verbose"):
    print(f"checked {path} ({os.environ.get('NOTES_FORMAT')})")
sys.exit(1 if long_lines else 0)
