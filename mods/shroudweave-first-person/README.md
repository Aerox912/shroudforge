# ShroudWeave First Person

Schaltet mit Home die Ego-Kamera und die korrigierte Ausdauerleistenposition gemeinsam um. Der Mod startet nach dem Laden in der Third-Person-Ansicht. Der Kameraabstand kann zwischen `-0.5` und `0.5` eingestellt werden. Standard ist der übernommene Wert `-0.15`.

Die native Steuerung gehört zum ShroudForge-Laufzeitmodul. Sie lässt nur das exakt identifizierte Client-Abbild mit den Kamera- und vierzehn Ausdauerleisten-Guards zu. Der Kamera-Hook ersetzt den internen Transform-Aufruf bei `0x2B2C92`; der Minimap-Hook am Kamera-System-Einstieg bleibt davon getrennt. Kamera und Ausdauerleiste lesen denselben atomaren Aktivierungswert. Der Loader installiert beide deaktivierten Weiterleitungs-Hooks einmal in seiner gehaltenen Startphase, wenn das Paket beim Start ausgewählt ist. Home ändert danach nur den gemeinsamen Aktivierungswert. Wird das Paket erst während einer laufenden Sitzung aktiviert, ist ein Neustart erforderlich.

Beim Deaktivieren werden beide Korrekturen abgeschaltet und die ursprünglichen nativen Funktionen weiter aufgerufen. Der Loader hält den deaktivierten Weiterleitungsstub bis zum Prozessende resident, damit bereits laufende Engine-Aufrufe keinen freigegebenen Code erreichen. Die Prüfung setzt weder eine allgemeine Widget-API noch Schreibzugriff auf ECS-Daten voraus.

## Kompatibilität

Die native Operation ist auf die vorhandene Client-Datei mit SHA-256 `d64966e23b1a458ee6f147bcbceac0b577fa7e39a212f371287d8cc119817882`, PE-Zeitstempel `0x6ad77f0c` und Bildgröße `0x2A1E000` begrenzt. Jeder Aufruf prüft außerdem die registrierte Kamera, den separaten Kamera-Transform-Aufruf und alle vierzehn statischen Ausdauerleisten-Guards. Ein anderer Build oder ein abweichender Guard wird geschlossen abgelehnt.

Die Implementierung wurde statisch und mit Offline-Fixtures geprüft. Sie wurde nicht in einem laufenden Spiel ausgeführt. Darum ist die Darstellung und Eingabe im Spiel noch nicht als akzeptiert bestätigt.

## Herkunft und Lizenz

Die Kamera- und Ausdauerleistenlogik stammt aus **First Person View** von s0T7x / Miguel Oppermann. Die MIT-Lizenz und die Urheberangabe liegen in [`LICENSE`](LICENSE). Die ShroudForge-Anpassung verlegt Eingabe, Besitz, Build-Prüfung und Code-Lebensdauer in den Loader.
