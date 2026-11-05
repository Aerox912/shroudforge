"""Retained server schema evidence only; never reads game originals or admits a producer."""
import hashlib
import json
from pathlib import Path
import sys


def load(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key")
            result[key] = value
        return result
    return json.loads(path.read_bytes(), object_pairs_hook=unique)


def pin(path):
    raw = path.read_bytes()
    return dict(path=str(path), bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()


def main():
    source, recipe_root, output = map(Path, sys.argv[1:])
    expected = {
        "types-enshrouded_server.json": (9196439, "fe74476f286d01fbd677b97abc6472ebb886313a7358ce0149cf1c6670bce4f3"),
        "types-enshrouded_server.identity.json": (635, "fbfbac8bfb9ff36944ac80ad1771273fc18677a2a8588931eaab2f86dce6d03d"),
        "PHASE-A-REPORT.json": (2561137, "98656f3b5f82b46e74dd5b7552c94a946476e1a707dc6cebcaa59325efec62b7"),
        "COMPLETE.json": (892, "bcdabf31eac66e0b85a83e14210ea8d1e90924eb75325a485ec02cc77334fcfb"),
    }
    pins = {}
    for name, (length, digest) in expected.items():
        pins[name] = pin(source / name)
        assert (pins[name]["bytes"], pins[name]["sha256"]) == (length, digest)
    registry = load(source / "types-enshrouded_server.json")
    report = load(source / "PHASE-A-REPORT.json")
    identity = load(source / "types-enshrouded_server.identity.json")
    complete = load(source / "COMPLETE.json")
    assert report["qualificationReady"] is False and complete["qualificationReady"] is False
    assert report["status"] == "unsupported_schema_presence"
    assert report["unresolvedNonzeroOptionalTypeReferences"] == 0
    assert report["referencePresenceIssues"] == []
    assert identity["registrySha256"] == pins["types-enshrouded_server.json"]["sha256"]
    assert registry["version"] == identity["pairedKfcGeneration"]
    assert complete["sourceInputs"] == report["sourceInputs"]
    for item in complete["files"]:
        assert (item["bytes"], item["sha256"]) == expected[item["name"]]
    rows, fingerprints = [], {}
    types = {t["index"]: t for t in registry["types"]}
    for ordinal, collision in enumerate(report["attributeNameCollisionRecords"], 1):
        ty = types[collision["typeIndex"]]
        before, after = collision["previous"], collision["current"]
        assert set(before) == set(after) and set(before) <= {"name", "namespace", "type", "value"}
        assert {k: v for k, v in before.items() if k != "value"} == {k: v for k, v in after.items() if k != "value"}
        name, where = after["name"], collision["map"]
        assert name in {"icon", "description", "enable_on", "server_only"}
        if where == "type.attributes":
            attributes = ty["attributes"]
        else:
            assert where.startswith("structFields.") and where.endswith(".attributes")
            attributes = ty["structFields"][where[len("structFields."):-len(".attributes")]]["attributes"]
        assert attributes[name] == after
        classification = "identical_represented_duplicate" if before == after else "different_string_value_only"
        if before != after:
            assert isinstance(before["value"], str) and isinstance(after["value"], str)
        rows.append(dict(auditRecordOrdinal=ordinal, owner=dict(typeIndex=ty["index"], qualifiedTypeName=ty["qualifiedName"], attributeMap=where, mapKey=name),
                         earlier=before, later=after, replacementSequence=[before, after], classification=classification, retainedRegistryWinner=after))
        fingerprints[str(ty["index"])] = dict(qualifiedName=ty["qualifiedName"], sha256=hashlib.sha256(canonical(ty)).hexdigest())
    assert len(rows) == 14 and len(fingerprints) == 11
    assert sum(row["classification"] == "identical_represented_duplicate" for row in rows) == 2
    recipe = load(recipe_root / "RECIPE.json")
    assert pin(recipe_root / "RECIPE.json")["sha256"] == "3bd7950844b905ddccd64907bc179584c91b22bd2a1b446304d10cd101e4b1c6"
    for name, stamp in recipe["effectiveFiles"].items():
        actual = pin(Path(recipe["environmentRoot"]) / name)
        assert (actual["bytes"], actual["sha256"]) == (stamp["bytes"], stamp["sha256"])
    output.mkdir(parents=True, exist_ok=False)
    ledger = dict(schemaVersion=1, scope="Exact captured server SDK-represented short-name map projection with complete represented replacement evidence", capturedOutputs=pins,
                  sourceInputs=report["sourceInputs"], generation=registry["version"], extractor=identity,
                  typeCount=len(types), records=rows, affectedTypeFingerprints=fingerprints,
                  representedReplacementSequenceCanonicalSha256=hashlib.sha256(canonical(rows)).hexdigest(),
                  limits=["No native precedence or raw pointer/alias identity is established", "No original raw interleaving or overwritten raw ordinals are established", "Map loses 12 earlier strings and 14 multiplicities; this ledger retains both represented values in audit order"],
                  originalExit=2, qualificationReady=False, producerAdmission=False, liveCompatibility=False)
    write(output / "LOSS-LEDGER.json", ledger)
    write(output / "INPUTS.json", dict(capturedOutputs=pins, recipe=pin(recipe_root / "RECIPE.json"), plan=pin(recipe_root / "PLAN-CHECK.json"), derivationSource=pin(Path(__file__))))
    print(json.dumps(dict(ledger=pin(output / "LOSS-LEDGER.json"), types=len(types), collisions=len(rows), owners=list(fingerprints.values()))))


def write(path, value):
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
