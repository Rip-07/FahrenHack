# FahrenHack

[![Build](https://github.com/Rip-07/FahrenHack/actions/workflows/build.yml/badge.svg?branch=main)](https://github.com/Rip-07/FahrenHack/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 2021](https://img.shields.io/badge/rust-edition%202021-orange.svg)](Cargo.toml)
[![Windows](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0078D6.svg)](#build)
[![Ko-fi](https://img.shields.io/badge/Ko--fi-support-FF5E5B.svg?logo=ko-fi&logoColor=white)](https://ko-fi.com/fahrentech)

Cheats for single-player PC games in one console-style app: ready game profiles, a guided setup that finds each value once and keeps working after game restarts, hotkeys and a phone remote over Wi-Fi. Written in Rust for Windows 10/11.

[Русская версия ниже](#русский)

> **Single-player only.** FahrenHack refuses to attach to games with anti-cheat, never bypasses copy protection and never changes game files on disk. It is not affiliated with any game developer or publisher; game names are used only to say what is supported. See [LEGAL.md](LEGAL.md).

## Features

- **Game library**: tiles with covers from your local Steam cache (not shipped with the project); shows which games are installed and running and starts them through Steam.
- **Ready profiles**: Resident Evil 4 (2023) and Schedule I (experimental).
- **Auto-attach**: FahrenHack connects by itself when a game with a profile starts.
- **Guided setup**: each cheat is found once on your game version in 1–2 minutes; a pointer scan then keeps it working after restarts.
- **Cheat controls**: switches, Apply buttons, numbers with − / +, sliders for multipliers, option lists, two hotkeys per cheat (up / down), tooltips, collapsible categories and pinned cheats.
- **Code injection**: profiles can patch game code in memory through a code cave; the original bytes are restored when the cheat is switched off.
- **Console-style UI**: frameless window, full screen (F11), keyboard and gamepad navigation (A select, B back, LB/RB sections, Y all cheats off).
- **Global hotkeys** work while the game has focus: `F1`, `CTRL+F2`, `SHIFT+NUM5`, …
- **Phone remote**: the same UI in your phone's browser over Wi-Fi, opened with a QR code. Off by default.
- **Memory scanner for any game**: exact and unknown-value search (memory snapshot), increased / decreased / changed / unchanged filters, save results as your own cheats.

## Quick start

1. Download `fahrenhack.exe` from Releases or [build it](#build).
2. Run it **as administrator**: Windows does not let other programs open the memory of many games otherwise.
3. Start the game. It shows up on the home screen with a LIVE badge and FahrenHack attaches.
4. Open **Cheats**. For a cheat that is not set up yet press **Set up** and follow the steps:
   - a **visible number** (money, ammo): enter it → Search → change it in the game → enter the new number → Filter;
   - a **hidden value** (health bar): Snapshot → take damage → Decreased → wait → Unchanged → heal → Increased.

   When a few addresses are left, test them with **Write** and press **This is it**.
5. Switch cheats on with the switch or a hotkey.

## How profiles work

1. A profile (`games/*.json`) lists the cheats and how to find them: setup hints, value type, default value, hotkeys.
2. The guided setup finds the value once on your PC; a pointer scan builds chains like `re4.exe+0x…→0x…→0x…` and stores them **per game build** (taken from the exe header).
3. On the next launch the address is resolved from the chain. Setting a cheat up again in a new session keeps only the chains that were confirmed.
4. Confirmed chains can be added to the profile's `builds` section, so everyone with the same game build gets the cheat without setup.

Profiles can also use byte signatures (`aob`), fixed chains (`pointer`), byte patches (`patch`) and code injection (`inject`). See [CONTRIBUTING.md](CONTRIBUTING.md).

## Phone remote

**Remote** → **Turn on remote**, then scan the QR code with a phone on the same Wi-Fi. If Windows asks about the firewall, allow private networks. The link contains a secret key; **New key** invalidates old links.

## Build

Requires Windows 10/11 x64, [Rust](https://rustup.rs) (MSVC toolchain) and Visual Studio Build Tools with the Windows SDK. WebView2 is part of Windows 11.

```bash
cargo build --release
```

The result is `target\release\fahrenhack.exe`; its icon is drawn from `src/logo.rs` at build time.

| Command | What it does |
|---|---|
| `fahrenhack` | desktop window |
| `fahrenhack --browser` | UI in your regular browser |
| `fahrenhack --headless` | server only, link in the console |

Data lives in `%APPDATA%\FahrenHack` (settings, calibrations, own cheats, own profiles); override it with `FAHRENHACK_DATA`.

## Development

```bash
cargo test
powershell -ExecutionPolicy Bypass -File tools\e2e.ps1
powershell -ExecutionPolicy Bypass -File tools\window-check.ps1
```

`e2e.ps1` runs the cheat engine against the test game in `tools/dummy-game` (calibration, pointer scan, freeze, code injection, restart); `window-check.ps1` checks the real WebView2 window. CI (GitHub Actions) runs the tests and builds the release exe with a SHA-256 checksum.

| Path | Contents |
|---|---|
| `src/mem.rs` | process memory: read, write, regions, modules, thread stacks |
| `src/scan.rs` | value scanner: exact search, memory snapshot, filters |
| `src/pointer.rs` | pointer chains and pointer scan |
| `src/inject.rs` | code injection through a code cave |
| `src/aob.rs` | byte signatures |
| `src/profile.rs` | game profiles, calibrations, own cheats |
| `src/app/` | game session, cheat operations, calibration, settings, background workers |
| `src/web/` | local HTTP API, token check, embedded UI |
| `src/window.rs` | WebView2 window (tao + wry) |
| `src/ui/` | UI for the window and the phone |
| `games/` | built-in game profiles |
| `tools/` | test game, end-to-end test, window check |
| `AGENTS.md`, `docs/STATUS.md` | notes for coding agents and the current project status |

## License

MIT — see [LICENSE](LICENSE). Game and company names belong to their owners and are used only to say what is supported.

---

## Русский

Читы для одиночных игр на ПК в одном приложении в стиле игровой консоли: готовые профили игр, мастер настройки, который один раз находит значение и дальше работает после перезапуска игры, горячие клавиши и пульт на телефоне по Wi-Fi. Написано на Rust для Windows 10/11.

> **Только для одиночных игр.** FahrenHack не подключается к играм с античитом, не обходит защиту от копирования и не меняет файлы игры на диске. Проект не связан с разработчиками и издателями игр; названия игр указаны только для совместимости. Подробнее в [LEGAL.md](LEGAL.md#русский).

### Возможности

- **Библиотека игр**: плитки с обложками из вашего локального кэша Steam (в проект они не входят); видно, какие игры установлены и запущены, можно запустить игру через Steam.
- **Готовые профили**: Resident Evil 4 (2023) и Schedule I (экспериментальный).
- **Автоподключение**: FahrenHack сам подключается, когда запускается игра с профилем.
- **Мастер настройки**: каждый чит один раз находится на вашей версии игры за 1–2 минуты, а поиск указателей сохраняет его на будущие запуски.
- **Элементы читов**: переключатели, кнопки «Применить», числа с «−» и «+», ползунки для множителей, списки вариантов, по две клавиши на чит («больше» и «меньше»), подсказки, сворачиваемые категории и закреплённые читы.
- **Инъекция кода**: профиль может менять код игры в памяти через код-пещеру; при выключении чита исходные байты возвращаются.
- **Консольный интерфейс**: окно без рамки, полный экран (F11), управление клавиатурой и геймпадом (A — выбрать, B — назад, LB/RB — разделы, Y — выключить все читы).
- **Горячие клавиши** работают прямо в игре: `F1`, `CTRL+F2`, `SHIFT+NUM5` и т.п.
- **Пульт на телефоне**: тот же интерфейс в браузере телефона по Wi-Fi, вход по QR-коду. По умолчанию выключен.
- **Сканер для любой игры**: точный поиск и поиск неизвестного значения через снимок памяти, отсев «больше / меньше / изменилось / не изменилось», сохранение найденного как своих читов.

### Быстрый старт

1. Скачайте `fahrenhack.exe` из Releases или соберите сами (см. «Сборка»).
2. Запустите **от имени администратора**: иначе Windows не даст открыть память многих игр.
3. Запустите игру: она появится на главном экране с меткой LIVE, FahrenHack подключится сам.
4. Откройте «Читы». У ненастроенного чита нажмите **«Настроить»** и следуйте подсказкам:
   - **видимое число** (деньги, патроны): введите число → «Искать» → измените его в игре → введите новое → «Отсеять»;
   - **скрытое значение** (полоска здоровья): «Снимок» → получите урон → «Уменьшилось» → подождите → «Не изменилось» → вылечитесь → «Увеличилось».

   Когда останется несколько адресов, проверьте их кнопкой «Записать» и нажмите «Это оно».
5. Включайте читы переключателем или горячей клавишей.

### Как работают профили

1. Профиль (`games/*.json`) описывает читы и способ их найти: подсказки мастера, тип значения, значение по умолчанию, клавиши.
2. Мастер один раз находит значение на вашем ПК, поиск указателей строит цепочки вида `re4.exe+0x…→0x…→0x…` и сохраняет их **для конкретной сборки игры** (по заголовку exe).
3. При следующем запуске адрес вычисляется по цепочке. Повторная настройка в новом сеансе оставляет только подтверждённые цепочки.
4. Подтверждённые цепочки можно добавить в раздел `builds` профиля: тогда у всех с той же сборкой игры чит заработает без настройки.

Профиль может использовать также сигнатуры (`aob`), фиксированные цепочки (`pointer`), замену байтов (`patch`) и инъекцию кода (`inject`). См. [CONTRIBUTING.md](CONTRIBUTING.md#русский).

### Пульт на телефоне

«Пульт» → «Включить пульт», затем отсканируйте QR-код телефоном в той же сети Wi-Fi. Если Windows спросит про брандмауэр, разрешите частные сети. Ссылка содержит секретный ключ; «Новый ключ» отключает старые ссылки.

### Сборка

Нужны Windows 10/11 x64, [Rust](https://rustup.rs) (toolchain MSVC) и Visual Studio Build Tools с Windows SDK. WebView2 уже есть в Windows 11.

```bash
cargo build --release
```

Готовый файл — `target\release\fahrenhack.exe`. Параметры запуска: `--browser` (интерфейс в обычном браузере), `--headless` (только сервер). Данные лежат в `%APPDATA%\FahrenHack`, путь меняется переменной `FAHRENHACK_DATA`.

### Разработка

```bash
cargo test
powershell -ExecutionPolicy Bypass -File tools\e2e.ps1
powershell -ExecutionPolicy Bypass -File tools\window-check.ps1
```

`e2e.ps1` проверяет движок на тестовой «игре» `tools/dummy-game`, `window-check.ps1` — настоящее окно WebView2. CI (GitHub Actions) запускает тесты и собирает exe с контрольной суммой SHA-256.

### Лицензия

MIT — см. [LICENSE](LICENSE). Названия игр и компаний принадлежат их владельцам и указаны только для совместимости.
