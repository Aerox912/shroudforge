# HD-Map-Quellstand auf ShroudForge 1.7.5

Task ID: `4.5.8-SHROUDFORGE-SOURCE-MAIN-01`

Dieser Integrationsstand beginnt unverändert bei ShroudForge 1.7.5,
Commit `46b59da1b3642f91fee3808c513efb996ec33f23`, Tag
`v1.7.5-build.39957852083`. Er übernimmt ausschließlich die anhand der
aufbewahrten Manifestkette belegten Quelldifferenzen. Vorhandene Dateien,
die in den Teilkopien fehlen, werden nicht gelöscht. Die vollständige
Dateiliste und die SHA-256-Werte stehen in
[hd-map-1.7.5-source-provenance.json](hd-map-1.7.5-source-provenance.json).

Der Vorgänger enthält bereits die Current-Assets-Transaktionen,
Startvorbereitung, Minimap-Overlay und Native-Plugin-Schnittstellen. Diese
Abhängigkeiten gehören zum dokumentierten Quellstand der DLLs. Sie wurden
zusammen mit dem Zoom- und Layout-Migrationsdelta übernommen. Ein Import
nur der zuletzt geänderten HD-Map-Dateien würde diesen Quellstand nicht
wiedergeben. Quellcode, Tests und vorhandene Prüfprogramme wurden unverändert
übernommen. Neue Tests oder Verhaltensänderungen wurden nicht hinzugefügt.

Das Parser-Submodul verwendet den autorisierten Fork
`https://github.com/Aerox912/kfc-parser.git`, Referenz `main`, mit dem
ausgewählten Commit `c7c9b72a60ac43d5c5fc4b1ace6e9a2fdaf20934`. Dieser
veröffentlichte Commit enthält die drei belegten Änderungen gegenüber
`2cf45e36cd4575fa3867761864a36e1883c82add` in
`crates/kfc-base/src/container/reader.rs`,
`crates/kfc-base/src/container/writer.rs` und
`crates/mod-loader-bridge/src/lib.rs`. Alle drei stimmen inhaltlich mit dem
aufbewahrten Snapshot überein. Reader und Writer unterscheiden sich bei den
rohen Hashes ausschließlich durch CRLF statt LF. Die Bridge stimmt auch
byteweise überein. Historische Snapshot-Hashes bleiben im JSON erhalten.
Der übergeordnete Auftrag setzt und prüft den Gitlink und den sauberen
Submodul-Checkout auf genau diesen Commit.
`dev/parser/prepare_parser.ps1` akzeptiert zusätzlich genau diesen Commit
als integrierten Parserstand. Die bisherigen unterstützten Pins bleiben
erhalten.

Das nlohmann-Submodul bleibt unverändert auf
`55f93686c01528224f448c19128836e7df245f72`. Header und MIT-Lizenz stimmen
exakt mit den Git-Blobs dieses Pins überein. Abweichende Checkout-Hashes
entstehen dort ausschließlich durch CRLF statt LF. Upstream-Credits,
MIT-Lizenzen und die Khronos-Hinweise bleiben erhalten.

Alle 1.325 Dateien des abschließenden Snapshots stimmen mit der verketteten
Inventarliste überein. Bei der Rekonstruktion stimmten 1.324 Dateien inhaltlich
überein. Das spätere Benennungsdelta am Buildprojekt ist unten dokumentiert.
Die erzeugte Datei `build/ui/modloader/index.html` wurde nicht als
Quellcode importiert. Ihr historischer Build-Eingangshash ist im JSON
festgehalten. Die vorhandenen Prüfprogramme enthalten teilweise Pfade zu
privaten, aufbewahrten Fixtures. Sie wurden hier nicht ausgeführt.

Die historische Runtime-DLL hat SHA-256
`afd8e1cd91583e51a6aa620e7b19590c1a6ad94a58d37724e6bcb69357c9a5f2`,
die Bootstrap-DLL
`9129152729426e0a683034fef472989f300e213f2422433f1eacf18e5a8167c3`.
Diese Hashes identifizieren vorhandene Artefakte. Sie belegen keinen Build
aus einem zukünftigen Integrationscommit. Source-Commit, Parser-Pin und
erfolgreicher CI-Build müssen vor einer solchen Zuordnung ergänzt werden.

## Separater Assets-Bootstrap

`dev/build/build_windows_bootstrap.ps1` baut dasselbe ausgewählte
1.7.5-C++-Projekt zweimal als `Release|x64`. Der normale Aufruf verwendet
`BootstrapTargetName=winmm`. Der zweite verwendet
`BootstrapTargetName=shroudforge-assets-bootstrap`. Beide Ziele besitzen
getrennte Objektverzeichnisse. Das zweite Ziel erzeugt
`build/x64/shroudforge-assets-bootstrap.dll` direkt aus den Quellen.

`dev/build/package_release.ps1` behält die normale `winmm.dll` im Paketroot
und nimmt die separat benannte DLL unter
`shroudforge/components/shroudforge-assets-bootstrap.dll` in das bestehende
Archiv `build/shroudforge-<version>-<build>.zip` auf. Der bestehende
CI-Artefaktpfad `build/shroudforge-*.zip` umfasst dieses Archiv weiterhin.
Die Datei `shroudforge/components/assets-bootstrap.json` enthält nach einem
tatsächlichen Paketlauf Quellversion, Git-Commit, 1.7.5-Basis, Buildziel,
Dateizuordnung und den dann berechneten DLL-Hash. Die MIT-Lizenz liegt im
selben Komponentenverzeichnis.

Für die aktuelle kombinierte Auswahl wird ausschließlich diese Komponente
unter dem Hostnamen `shroudforge-assets-bootstrap.dll` explizit geladen.
Die vorhandene Shroudtopia-`winmm.dll` im Spielroot bleibt erhalten. Die
normale ShroudForge-`winmm.dll` aus dem Standardpaket gehört nicht zusätzlich
in diese Auswahl. Der Komponentenpfad im ZIP ist kein Installationsauftrag.
Die Lade- und Installationszuordnung bleibt beim übergeordneten Auftrag.

Die neue Benennung und Paketierung ist ein nachträgliches Integrationsdelta
gegenüber dem historischen Snapshot. Es wurde noch kein Build ausgeführt.
Für die neu benannte DLL wird deshalb kein Binärhash und keine Gleichheit
mit der historischen `bootstrap-proxy.dll` behauptet.

Durchgeführt wurden Quell-, Manifest- und Hashvergleiche. Tests, Builds,
CI, Commit, Push, Installation und Spieltests wurden nicht durchgeführt.
Die vorhandenen schmutzigen Checkouts und die Live-Installation bleiben
unverändert. Es besteht keine Aussage zu Rendering oder Multiplayer.

Der isolierte Worktree bleibt als unveröffentlichter Prüfstand für Delivery
erhalten. Er darf erst nach geprüfter Quellauslieferung und Freigabe dieses
Checkpoints durch den übergeordneten Auftrag entfernt werden. Bestehende
Build-Belege und fremde Arbeitsverzeichnisse gehören nicht zu seiner
Bereinigung.
