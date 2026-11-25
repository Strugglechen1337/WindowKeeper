# WindowKeeper

WindowKeeper ist eine kleine, **portable Windows-Tray-App**, die sich merkt, wo Ihre Fenster liegen (Position, Größe, maximiert/minimiert, Monitor) und sie nach dem Start oder Login automatisch wieder dorthin zurücksetzt. Eine einzelne EXE, kein Installer, keine Registry-Einträge (außer dem optionalen Autostart).

## Funktionen

- Merkt sich Position, Größe, Zustand (normal/maximiert/minimiert) und Monitor sichtbarer Anwendungsfenster.
- **Automatisch speichern** (Standard: an): Das Layout wird gespeichert, sobald Sie ein Fenster verschoben/skaliert haben oder sich sein Zustand ändert (WinEvent-Hooks, entprellt, plus ein vorsichtiger Hintergrund-Scan).
- **Automatisch wiederherstellen** beim Start/Login. Fenster, die erst später erscheinen, werden einige Minuten lang (Standard: 5) ebenfalls platziert.
- Fenster werden über *Pfad der EXE + Fensterklasse* erkannt, bei generischen Klassen oder mehrdeutigen Treffern zusätzlich über den Fenstertitel.
- Ignoriert werden Tool-Fenster, verdeckte („cloaked“) Fenster, UWP-Geisterfenster, hängende oder unsichtbare Fenster sowie WindowKeeper selbst.
- **Multi-Monitor-fähig** und DPI-bewusst (Per-Monitor-DPI v2). Fehlt der gespeicherte Monitor (z. B. zweites Display abgesteckt), landet das Fenster vollständig sichtbar im Arbeitsbereich des Hauptmonitors.
- **Übersicht (GUI)** im Windows-11-Stil (hell/dunkel automatisch): Status, gemerkte Fenster, Einstellungen und Updates – siehe unten.
- **Selbst-Updater** über GitHub Releases mit SHA256-Prüfung.
- Es werden **keine Programme gestartet** – WindowKeeper positioniert nur Fenster, die bereits laufen.

## Download und Benutzung

1. `WindowKeeper.exe` von der [Releases-Seite](https://github.com/Strugglechen1337/WindowKeeper/releases) herunterladen.
2. In einen beliebigen beschreibbaren Ordner legen (z. B. `C:\Tools\WindowKeeper\`) und starten.
3. Beim ersten Start erscheint ein Hinweis, dass WindowKeeper im Infobereich der Taskleiste läuft (ggf. hinter dem Pfeil „Ausgeblendete Symbole“).

### Übersicht (Oberfläche)

- **Öffnen:** Linksklick (oder Doppelklick) auf das Tray-Symbol oder im Rechtsklick-Menü **Übersicht öffnen**. Ist das Fenster schon offen, wird es nur nach vorn geholt.
- **Schließen:** Das Schließen des Fensters beendet WindowKeeper **nicht** – die App läuft im Tray weiter. Beenden geht nur über das Tray-Menü.
- **Übersicht:** Status, Zeitpunkt der letzten Speicherung/Wiederherstellung, Anzahl gemerkter und ignorierter Fenster, Schaltflächen *Layout speichern* / *Layout wiederherstellen*.
- **Fenster:** Liste aller gemerkten Fenster (Programmsymbol, Datei, Titel, Position/Größe, Monitor, Zustand) mit Suche. Pro Eintrag:
  - **Jetzt wiederherstellen** setzt nur dieses Fenster zurück (läuft das Programm nicht, erscheint ein Hinweis – es wird nichts gestartet).
  - **Ignorieren** lässt den Eintrag in der Liste (abgeblendet), aber er wird beim automatischen Wiederherstellen, bei *Layout wiederherstellen* und beim automatischen Speichern übersprungen bzw. nicht verändert. Der Schalter wird gespeichert.
  - **Löschen** entfernt den Eintrag nach Rückfrage. Ist das Fenster noch geöffnet, wird es beim nächsten Speichern erneut gemerkt – zum dauerhaften Ausschließen *Ignorieren* nutzen.
- **Einstellungen:** Autostart, automatisches Speichern, Verzögerung/Prüfintervall, Beobachtung später Fenster, Update-Prüfung beim Start. Änderungen werden sofort in `data/config.toml` gespeichert.
- **Info:** Version, GitHub-Link, Lizenz und *Nach Updates suchen* (Installation nur nach Rückfrage, SHA256-Prüfung Pflicht).

Die Oberfläche braucht OpenGL 2.0 (praktisch jeder Grafiktreiber). Ohne läuft WindowKeeper im Tray weiter und meldet das.

### Tray-Menü

| Eintrag | Wirkung |
| --- | --- |
| Übersicht öffnen | Öffnet die Oberfläche (auch per Linksklick auf das Symbol) |
| Layout jetzt speichern | Speichert alle aktuellen Fensterpositionen sofort |
| Layout jetzt wiederherstellen | Setzt gespeicherte Positionen sofort um |
| Automatisch speichern an/aus | Schaltet das automatische Speichern um (wird gespeichert) |
| Mit Windows starten an/aus | Setzt/entfernt den Autostart-Eintrag |
| Nach Updates suchen | Prüft sofort auf eine neue Version |
| Beenden | Beendet WindowKeeper |

Ist noch kein Layout gespeichert, passiert beim Wiederherstellen nichts (es erscheint nur ein kurzer Hinweis).

## Portabel: der Ordner `data/`

Alle Einstellungen und Daten liegen im Ordner `data/` **neben der EXE** (wird beim ersten Start angelegt):

| Datei | Inhalt |
| --- | --- |
| `data/config.toml` | Einstellungen (TOML) |
| `data/layout.json` | Gespeichertes Fensterlayout (inkl. Ignorieren-Markierung) |
| `data/state.json` | Zeitpunkt der letzten Speicherung/Wiederherstellung |
| `data/windowkeeper.log` | Logdatei (auf ca. 512 KB begrenzt, rotiert nach `.log.1`) |

Nichts wird unter `%APPDATA%` abgelegt. Ist der EXE-Ordner nicht beschreibbar, weicht WindowKeeper als Notlösung auf den Temp-Ordner aus (im Log vermerkt). Zum „Deinstallieren“ genügt es, die EXE und den Ordner `data/` zu löschen (vorher den Autostart ausschalten).

### Einstellungen (`data/config.toml`)

```toml
auto_save = true              # automatisch speichern
restore_watch_minutes = 5     # so lange werden spät erscheinende Fenster platziert (0 = aus)
debounce_ms = 1500            # Entprellung des automatischen Speicherns
scan_interval_secs = 30       # Intervall des Hintergrund-Scans
check_updates_on_start = true # beim Start höchstens einmal täglich nach Updates suchen
last_update_check = 0         # wird automatisch gepflegt
first_run_done = false        # wird automatisch gepflegt
```

## Autostart

„Mit Windows starten an/aus“ schreibt den Wert `WindowKeeper` unter
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (Pfad zur aktuellen EXE, in Anführungszeichen). Das ist die **einzige** Stelle, an der WindowKeeper die Registry benutzt. Verschieben Sie die EXE, wird der Pfad beim nächsten Start automatisch korrigiert, sofern der Autostart aktiv ist.

## Updates

- Beim Start prüft WindowKeeper höchstens **einmal pro Tag** auf eine neuere Version; „Nach Updates suchen“ prüft immer.
- Es wird das Windows-Programm aus dem neuesten GitHub-Release geladen und gegen die veröffentlichte SHA256-Prüfsumme (`WindowKeeper.exe.sha256`) verifiziert. **Fehlt die Prüfsumme oder passt sie nicht, wird abgebrochen.**
- Installiert wird **nie ohne Rückfrage** (Windows-Dialog auf Deutsch).
- Ablauf: Die laufende EXE wird zu `WindowKeeper.exe.old` umbenannt, die neue an ihre Stelle gelegt und neu gestartet; die `.old`-Datei wird beim nächsten Start gelöscht.

## Aus dem Quellcode bauen

Voraussetzung: [Rust](https://rustup.rs) (Edition 2021, stable).

```powershell
git clone https://github.com/Strugglechen1337/WindowKeeper.git
cd WindowKeeper
cargo build --release
# Ergebnis: target\release\windowkeeper.exe
```

Die Logik (Fenster-Matching, Monitor-Fallback, Versionsvergleich, Konfiguration, Updater-Parser) ist plattformunabhängig und lässt sich auch unter Linux testen:

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Die Oberfläche (egui/eframe, OpenGL) wird nur unter Windows gebaut; die Logik dahinter (`ui_model`, `layout`, `config`, `state`) ist plattformunabhängig und wird unter Linux getestet. Die CI (`.github/workflows/ci.yml`) lädt die gebaute EXE als Artefakt `WindowKeeper` (`WindowKeeper.exe`) hoch.

Ein Release entsteht, wenn ein Tag `vX.Y.Z` gepusht wird: Der Workflow baut die EXE und lädt `WindowKeeper.exe` sowie `WindowKeeper.exe.sha256` hoch.

## Bekannte Einschränkungen

- Fenster von Programmen mit erhöhten Rechten (Administrator) können nur erfasst werden, wenn WindowKeeper ebenfalls erhöht läuft.
- Es gibt ein Layout pro Rechner-Konfiguration; wird ein Monitor abgesteckt und Windows verschiebt Fenster, kann das beim automatischen Speichern übernommen werden.
- Manche Anwendungen (z. B. UWP-Apps) positionieren sich nach dem Start selbst und ignorieren die Wiederherstellung.

## Ideen für die Zukunft

- Optional die gespeicherten Anwendungen selbst automatisch starten (aktuell bewusst **nicht** implementiert).
- Mehrere benannte Layouts / ein Layout pro Monitor-Konfiguration.

## Lizenz

[MIT](LICENSE) – Copyright (c) 2026 Strugglechen1337
