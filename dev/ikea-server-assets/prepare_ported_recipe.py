"""Prepare current ported server asset sources with retained IKEA settings.

Only explicit mod sources are read. This does not run the mods or qualify them.
"""
import hashlib
import importlib.util
import json
import pathlib

HERE = pathlib.Path(__file__).parent
spec = importlib.util.spec_from_file_location("recipe", HERE / "prepare_recipe.py")
recipe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recipe)
BASE = pathlib.Path(r"E:\Build\ikea-server-asset-recipe-20261105-r1")
CLIENT = pathlib.Path(r"E:\Build\sf178-generation-control-inputs-20261105-r1\environment")
CANDIDATE = pathlib.Path(r"E:\Build\sf178-full8k-candidate-20261105-r1\INPUT-MANIFEST-r2.json")
CANDIDATE_SHA = "f8a5630e76436d8eb226d346654df9a6204e995150610c0fd12528f2e4267dda"
OUTPUT = pathlib.Path(r"E:\Build\ikea-server-ported-recipe-20261105-r2")
PORTED = ["aldrich-witcher-signs", "xhl-rested-enhanced", "xhl-workshop-speed-20x",
          "esm.buff-refresh", "esm.build-zones", "esm.magic-factories",
          "esm.magic-furniture", "esm.shroud-building", "esm.stack-size"]
RETAINED = ["godmode_spawn_variants", "XHL-Vein-Mining",
            "XHL-Unlimited-Gifting-Range", "XHL-Grappling-Hook-Pull-2x"]


def pin(path):
    raw = recipe.ordinary(path).read_bytes()
    return {"path": str(path), "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}


def prepare():
    if OUTPUT.exists():
        raise ValueError("fresh output required")
    recipe.ordinary(OUTPUT.parent)
    candidate_pin = pin(CANDIDATE)
    if candidate_pin["sha256"] != CANDIDATE_SHA:
        raise ValueError("candidate identity differs")
    candidate = json.loads(CANDIDATE.read_bytes())
    base = json.loads((BASE / "RECIPE.json").read_bytes())
    if base["sourceJoinSha256"] != recipe.JOIN_SHA256:
        raise ValueError("historical recipe differs")
    components = next(value for value in candidate.values()
                      if isinstance(value, list) and value and isinstance(value[0], dict)
                      and "componentId" in value[0] and "sourceFiles" in value[0])
    by_id = {c["componentId"]: c for c in components}
    files, sources, changes = {}, {}, []
    for module in PORTED:
        component = by_id[module]
        if component["sourceRoot"] is not None or component["overlays"]:
            raise ValueError("unexpected ported source override")
        for member, identity in component["sourceFiles"].items():
            name = f"mods/{module}/{member}"
            recipe.relative(name)
            p = CLIENT / name
            source_pin = dict(identity, path=str(p))
            files[name] = recipe.read_pin(source_pin)
            sources[name] = source_pin
        extension_name = f"mods/{module}/extended.mod.json"
        extension = json.loads(files[extension_name])
        before = extension["targets"]
        extension["targets"] = ["server"]
        files[extension_name] = (json.dumps(extension, indent=2) + "\n").encode()
        changes.append({"file": extension_name, "field": "targets", "before": before,
                        "after": ["server"], "meaning": "private preparation target, not compatibility acceptance"})
    for name, identity in base["files"].items():
        if name.split("/")[1] not in RETAINED:
            continue
        source_pin = dict(identity, path=str(BASE / "environment" / name))
        files[name] = recipe.read_pin(source_pin)
        sources[name] = source_pin
    for module in RETAINED:
        name = f"mods/{module}/extended.mod.json"
        if name in files:
            raise ValueError("unexpected retained extension")
        files[name] = (json.dumps({"schemaVersion": 1, "enabled": True,
            "targets": ["server"], "launcher": "EML", "settings": {}}, indent=2) + "\n").encode()
        changes.append({"file": name, "reason": "explicit server target; legacy missing extension defaults to client"})
    for old, new in [("XHL-Rested-Enhanced", "xhl-rested-enhanced"),
                     ("XHL-Workshop-Speed-20x", "xhl-workshop-speed-20x")]:
        old_name, name = f"mods/{old}/config.txt", f"mods/{new}/config.txt"
        source_pin = dict(base["files"][old_name], path=str(BASE / "environment" / old_name))
        files[name] = recipe.read_pin(source_pin)
        sources[name] = source_pin
        changes.append({"file": name, "reason": "exact active IKEA config bytes preserved"})
    stack = json.loads(files["mods/esm.stack-size/extended.mod.json"])
    if stack["settings"]["maxStackSize"]["value"] != 65535:
        raise ValueError("stack setting differs")
    stack_text = (BASE / "environment/mods/XHL-Max-Stack-65535/config.txt").read_text()
    if not all(line in stack_text.splitlines() for line in
               ["enabled=true", "maxStackSize=65535", "preserveSingleStackItems=true"]):
        raise ValueError("IKEA stack config differs")
    if len({name.split('/')[1] for name in files}) != 13:
        raise ValueError("expected13independent asset modules")
    if any(name.lower().endswith((".dll", ".exe", ".bak")) for name in files):
        raise ValueError("native or backup member excluded")
    OUTPUT.mkdir()
    environment = OUTPUT / "environment"
    environment.mkdir()
    for name, body in sorted(files.items()):
        p = environment / recipe.relative(name)
        p.parent.mkdir(parents=True, exist_ok=True)
        with p.open("xb") as stream:
            stream.write(body)
        if p.read_bytes() != body:
            raise ValueError("output verification failed")
    report = {
        "schemaVersion": 1, "status": "PORTED_SERVER_RECIPE_PREPARED_NOT_QUALIFIED",
        "candidate": candidate_pin, "historicalRecipe": pin(BASE / "RECIPE.json"),
        "environmentRoot": str(environment), "sourceFiles": sources,
        "effectiveFiles": {name: pin(environment / name) for name in sorted(files)},
        "changes": changes,
        "preservedBehaviorMapping": {
            "Enable_MagicFurniture=true": "esm.magic-furniture",
            "Enable_MagicFactories=true": "esm.magic-factories",
            "Enable_BuffReapplication=true": "esm.buff-refresh",
            "Enable_BuildingTweaks=true": "esm.build-zones",
            "Enable_PlacementTweaks=true; BuildInFog=true; NoBuildZoneNeeded=false; SafetySkip=Material": "esm.shroud-building",
            "XHL stack65535; preserveSingleStackItems=true; EMBER stackfalse": "esm.stack-size is sole active owner"
        },
        "nativeLibrariesIncluded": False, "gameBodiesRead": False,
        "liveChanges": False, "backupsCreated": False,
        "serverPlannerVerified": False, "latestServerGenerationAccepted": False,
        "retention": "Main-owned staged ported recipe until integrated server acceptance; historical input retained for continuity."
    }
    with (OUTPUT / "RECIPE.json").open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
    return {"files": len(files), "bytes": sum(map(len, files.values())), "receipt": pin(OUTPUT / "RECIPE.json")}


if __name__ == "__main__":
    print(json.dumps(prepare()))
