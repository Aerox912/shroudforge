"""Stage the pinned IKEA asset recipe from retained mod sources, never game files.

This creates a private preparation input, not a deployment or compatibility result.
The historical backup and root evidence documents are deliberately not packaged.
"""
import argparse
import hashlib
import json
import pathlib
import stat
import zipfile

JOIN_SHA256 = "e7e0c424b7a16abbea8f6bfd48700578fa20c98203f333f859aed64a8c9e7e71"


def ordinary(path):
    path = pathlib.Path(path)
    for part in [path, *path.parents]:
        info = part.lstat()
        if part.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError("linked input/output ancestor")
    return path


def read_pin(pin):
    path = ordinary(pin["path"])
    if not stat.S_ISREG(path.stat().st_mode):
        raise ValueError("non-file input")
    raw = path.read_bytes()
    if len(raw) != pin["bytes"] or hashlib.sha256(raw).hexdigest() != pin["sha256"]:
        raise ValueError("retained input differs: " + str(path))
    return raw


def relative(name):
    parts = name.split("/")
    if (len(parts) < 3 or parts[0] != "mods" or
            any(p in ("", ".", "..") or p.endswith((" ", ".")) for p in parts) or
            any(c in name for c in "\\:<>\"|?*")):
        raise ValueError("invalid package member")
    return pathlib.Path(*parts)


def prepare(join_path, output):
    raw = ordinary(join_path).read_bytes()
    if hashlib.sha256(raw).hexdigest() != JOIN_SHA256:
        raise ValueError("source recipe join differs")
    join = json.loads(raw)
    output = pathlib.Path(output)
    if not output.is_absolute() or output.drive.upper() != "E:":
        raise ValueError("private E output required")
    if any(part in (".", "..") for part in output.parts):
        raise ValueError("noncanonical output")
    ordinary(output.parent)
    if output.exists():
        raise ValueError("fresh output required")

    files, provenance, excluded = {}, {}, []
    for row in join["retainedSourceRows"]:
        name = row["relative"]
        if len(name.split("/")) == 2 or name.endswith(".bak"):
            excluded.append(name)
            continue
        relative(name)
        files[name] = read_pin(row["retainedSource"])
        provenance[name] = row["retainedSource"]
    for row in join["currentGodmodeSuccessor"]:
        name = row["relative"]
        if name not in files or not row["currentMatches"]:
            raise ValueError("unexpected successor")
        archive = read_pin(row["archive"])
        import io
        with zipfile.ZipFile(io.BytesIO(archive)) as package:
            matches = [i for i in package.infolist() if i.filename == row["member"]]
            if len(matches) != 1 or matches[0].file_size != row["bytes"]:
                raise ValueError("ambiguous or oversized successor")
            body = package.read(matches[0])
        if hashlib.sha256(body).hexdigest() != row["sha256"]:
            raise ValueError("successor differs")
        files[name], provenance[name] = body, row
    if len(files) != 83 or len({n.casefold() for n in files}) != len(files):
        raise ValueError("recipe membership differs")
    modules = sorted({n.split("/")[1] for n in files})
    if modules != join["historicalSelectedModules"]:
        raise ValueError("module membership differs")
    manifests = [json.loads(files[f"mods/{m}/mod.json"]) for m in modules]
    if set(m["id"] for m in manifests) != set(join["historicalExecutionOrder"]):
        raise ValueError("module identities differ")
    if any("patch" not in m["capabilities"] for m in manifests):
        raise ValueError("non-patch package")

    # Validate every source before any output is created. Failed output is retained.
    output.mkdir()
    environment = output / "environment"
    environment.mkdir()
    for name, body in sorted(files.items()):
        destination = environment / relative(name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as stream:
            stream.write(body)
        if destination.read_bytes() != body:
            raise ValueError("staging verification failed")
    receipt = {
        "schemaVersion": 1,
        "status": "PRIVATE_RECIPE_STAGED_NOT_GAME_QUALIFIED",
        "sourceJoinSha256": JOIN_SHA256,
        "environmentRoot": str(environment),
        "files": {name: {"bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()}
                  for name, body in sorted(files.items())},
        "provenance": provenance,
        "excludedHistoricalMembers": excluded,
        "expectedHistoricalOrder": join["historicalExecutionOrder"],
        "serverPlannerOrderVerified": False,
        "latestServerAssetsGenerated": False,
        "nativeLibrariesIncluded": False,
        "liveChanges": False,
        "backupsCreated": False,
        "retention": "Main-owned recipe input; retain until integrated server asset acceptance."
    }
    with (output / "RECIPE.json").open("x", encoding="utf-8") as stream:
        json.dump(receipt, stream, indent=2)
    return {"files": len(files), "bytes": sum(map(len, files.values())),
            "receipt": str(output / "RECIPE.json")}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("join", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    print(json.dumps(prepare(args.join, args.output)))
