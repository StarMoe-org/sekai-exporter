"""Checks that every dependency between workspace crates points to a lower layer.

The layers are the table in docs/architecture.md. Run from the repository root:

    python3 tools/check-layers.py
"""

import json
import subprocess
import sys

LAYERS = {
    "sekai-unity": 0,
    "sekai-format": 0,
    "sekai-layouts": 1,
    "sekai-codec": 1,
    "sekai-catalog": 1,
    "sekai-serve": 1,
    "sekai-game": 1,
    "sekai-convert": 2,
    "sekai-encode": 2,
    "sekai-publish": 2,
    "sekai-pipeline": 3,
    "sekai-moesekai": 3,
    "sekai-mysekai": 3,
    "sekai-exporter": 4,
    "sekai-wasm": 4,
}

metadata = json.loads(
    subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"])
)
workspace = {p["name"] for p in metadata["packages"]}
errors = [f"{name} is not in the layer table" for name in sorted(workspace - LAYERS.keys())]
errors += [f"{name} is in the layer table but not in the workspace" for name in sorted(LAYERS.keys() - workspace)]
for package in metadata["packages"]:
    for dep in package["dependencies"]:
        if dep["name"] in LAYERS and dep["kind"] != "dev":
            if LAYERS[dep["name"]] >= LAYERS[package["name"]]:
                errors.append(
                    f"{package['name']} (layer {LAYERS[package['name']]}) depends on "
                    f"{dep['name']} (layer {LAYERS[dep['name']]})"
                )
for error in errors:
    print(error)
sys.exit(1 if errors else 0)
