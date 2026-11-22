# Security

What FahrenHack does on your system and how to check that a downloaded exe is genuine.

[Русская версия ниже](#русский)

## Checking the exe

- Official builds are made by GitHub Actions from the sources in this repository (`.github/workflows/build.yml`), with a SHA-256 checksum next to each build.
- No packers or obfuscation: you can build it yourself with `cargo build --release`.
- FahrenHack **never asks you to disable your antivirus**. It does not download or run code from the internet, does not inject DLLs and does not install drivers.

## What it does on your system

- Reads and writes the memory of the one game process you attached to, with the standard `ReadProcessMemory` and `WriteProcessMemory`. Code-patch cheats change game code in memory only and restore it when switched off.
- Refuses to attach to processes with anti-cheat.
- Runs a local web server on `127.0.0.1`. The network is reachable only while the phone remote is on, and every request needs the secret key from the QR code (**New key** replaces it).
- Keeps settings and calibrations in `%APPDATA%\FahrenHack`. A memory snapshot for search is written to `%TEMP%` and deleted after use.

## Reporting a vulnerability

Please do not post vulnerability details in public issues. Use GitHub's private report: **Security → Report a vulnerability**.

---

## Русский

Что FahrenHack делает с системой и как проверить, что скачанный exe настоящий.

### Проверка exe

- Официальные сборки делает GitHub Actions из исходников этого репозитория (`.github/workflows/build.yml`), рядом с каждой сборкой — SHA-256.
- Без упаковщиков и обфускации: можно собрать самому командой `cargo build --release`.
- FahrenHack **никогда не просит отключить антивирус**. Он не скачивает и не запускает код из интернета, не внедряет DLL и не ставит драйверы.

### Что он делает с системой

- Читает и пишет память только того процесса игры, к которому вы подключились, через стандартные `ReadProcessMemory` и `WriteProcessMemory`. Читы с патчем кода меняют код игры только в памяти и возвращают его при выключении.
- Не подключается к процессам с античитом.
- Запускает локальный веб-сервер на `127.0.0.1`. Сеть доступна только при включённом пульте, и каждый запрос требует секретный ключ из QR-кода («Новый ключ» его меняет).
- Хранит настройки и калибровки в `%APPDATA%\FahrenHack`. Снимок памяти для поиска пишется во `%TEMP%` и удаляется после использования.

### Сообщить об уязвимости

Не публикуйте детали уязвимостей в открытых issue. Используйте приватный отчёт на GitHub: **Security → Report a vulnerability**.
