# Contributing

How to add a game or improve a profile. Profiles live in `games/*.json` and are compiled into the program. To try a profile without rebuilding, put the file into `%APPDATA%\FahrenHack\games\`: it replaces the built-in profile with the same `id`.

[Русская версия ниже](#русский)

## Profile rules

1. **Single-player games and modes only.** No profiles for online modes, games with anti-cheat, leaderboards or competitions.
2. **No paid content.** No cheats for DLC, microtransactions, real-money currency or license unlocks.
3. **No bypassing protection.** Patches (`patch`, `inject`) must not disable DRM, anti-tamper, integrity checks or anti-cheat. Game files on disk are never modified.
4. **Your own findings only.** Do not copy Cheat Engine tables, scripts or offsets from other trainers and forums; find values with the FahrenHack scanner. If a finding is someone else's, get the author's permission and credit them.
5. **No game assets.** No logos, covers, screenshots or texts from the game. Describe cheats in your own words.

## Profile format

```json
{
  "id": "mygame",
  "title": "Game title",
  "subtitle": "Year or edition",
  "processes": ["game.exe"],
  "steam_appid": 123456,
  "accent": "#2b4c6f",
  "notes": "Hint shown to the player",
  "pointer_scan": { "max_depth": 5, "max_offset": 4096 },
  "categories": [ { "name": "Player", "icon": "user" }, { "name": "Inventory", "icon": "bag" } ],
  "cheats": [
    {
      "id": "money",
      "category": "Inventory",
      "icon": "coin",
      "name": "Money",
      "description": "What the cheat does",
      "kind": "number",
      "vtype": "i32",
      "min": 0, "max": 9999999, "step": 1000,
      "hotkey": "F3", "hotkey_down": "SHIFT+F3",
      "locator": { "type": "calibrate", "mode": "exact", "prompt": "How much money do you have?", "steps": ["Step 1", "Step 2"] }
    }
  ],
  "builds": {}
}
```

`categories` is optional: it sets the order and icons of categories; categories not listed follow in order of appearance.

### Cheat fields

- `kind`, the control shown in the UI:

  | `kind` | UI | What it does |
  |---|---|---|
  | `freeze` | switch | keeps the value (or the code patch) while on |
  | `set` | Apply button | writes `value` once |
  | `number` | − / field / + | shows the game value, changes it by `step`, writes a typed value; limited by `min` and `max` |
  | `slider` | slider | setting from `min` to `max` in `step`s; `off_value` switches the cheat off. For multipliers, damage, speed |
  | `select` | list + Apply | writes the chosen option from `options` (`{ "label": "…", "value": "…" }`) |

- `vtype`: `i32`, `i64`, `f32`, `f64` or `u8` (one byte, for game flags; profiles only).
- `value`: default value. For `freeze`, `null` means "keep what it was when switched on"; for `slider` and `select`, the initial position.
- `off_value`: for `slider`, the value that means "off"; for `freeze`, the value written back when the cheat is switched off (a game flag returns to 0, a speed to 1).
- `hotkey`: e.g. `F1`, `CTRL+F2`, `SHIFT+NUM5`, `ALT+K`. For `number` and `slider` it means "up", for `select` "next option".
- `hotkey_down`: "down" or "previous option" (`number`, `slider`, `select` only).
- `info`: tooltip behind an "i". `warn`: warning behind a yellow "!" for side effects the player should know about.
- `icon`: `heart`, `ammo`, `coin`, `gem`, `knife`, `star`, `bolt`, `user`, `bag`, `chart`, `gun`, `skull`, `flag`, `run`, `flame`, `timer`, `key`, `shield`, `target`, `eye`.

### Finding the value (`locator`)

| `type` | Meaning |
|---|---|
| `calibrate` | The guided setup finds the value on the player's PC: `mode: exact` for a visible number, `unknown` for a bar. FahrenHack then builds pointer chains. |
| `pointer` | Fixed chain: `{ "module": "game.exe", "offsets": ["0x1234", "0x10", "0x8"] }`, i.e. `[[game.exe+0x1234]+0x10]+0x8`. |
| `aob` | Byte signature in a module. Optional `rip: [disp32 offset, instruction length]` for RIP-relative addressing, then `offsets` as in `pointer`. |
| `patch` | Byte replacement at a signature: `pattern`, `offset`, `bytes`. The original bytes are restored when switched off. |
| `inject` | Code injection: `size` bytes (at least 5) at the signature (`pattern` + `offset`) become a jump into a code cave near the module, where `code` runs. The original bytes are restored when switched off. |
| `item` | An item in an inventory list: `pattern`, `rip`, `offsets` lead (as in `aob`) to a managed `List<T>`; FahrenHack finds the element whose item object (at `item`, default `10`) has the u32 `id` at `id_at` (default `28`) and uses the value at `field`. |

### `code` template for `inject`

Hex bytes separated by spaces, plus placeholders:

| Placeholder | Inserted |
|---|---|
| `{orig}` | the original bytes overwritten by the jump. Do not use it if they contain RIP-relative addressing |
| `{return}` | jump back to the first instruction after the overwritten bytes (5 bytes) |
| `{param}` | rel32 to the parameter slot, for an instruction whose displacement is its last field, e.g. `F3 0F 59 05 {param}` = `mulss xmm0, [param]` |
| `{param_abs}` | 8-byte absolute address of the parameter, e.g. `48 B8 {param_abs}` = `mov rax, param` |
| `{addr:ID}` | rel32 to a slot that holds the current address of another cheat's value (cheat `ID` of the same profile), or 0 while it is unknown. FahrenHack keeps it current, e.g. to leave the player alone in a damage hook |

The parameter is the slider value (`slider`), the chosen option (`select`) or the switch's `value`, written as `vtype`. A slider changes it live without touching the code. Two cheats cannot patch the same code at once: the second one gets a clear error.

Example from `tools/e2e.ps1`, "damage per hit" for the test game, where `take_damage` compiles to `lock sub [rcx+18h], edx; ret`:

```json
{ "id": "dmg", "kind": "slider", "vtype": "i32", "min": 0, "max": 50, "step": 1, "off_value": "7",
  "locator": { "type": "inject", "pattern": "F0 29 51 18 C3 CC", "size": 5,
               "code": "8B 15 {param} F0 29 51 18 C3" } }
```

`mov edx, [param]` replaces the damage, followed by the original `lock sub` and `ret`.

### Verified chains (`builds`)

After a cheat is set up, FahrenHack stores its chains in `%APPDATA%\FahrenHack\calibration\<game>\<build>.json`. The game build is taken from its exe header and looks like `5f3a2b1c-1a2b000`.

A chain may be added to the profile once it is **confirmed in at least two game launches** (`confirmations` ≥ 2, and after a restart the cheat found the right value):

```json
"builds": {
  "5f3a2b1c-1a2b000": {
    "money": [ { "module": "game.exe", "offsets": ["0x2F1A8C0", "0x18", "0x2C4"] } ]
  }
}
```

With this section the cheat works on that build right away, without setup.

## Before a pull request

```bash
cargo test
powershell -ExecutionPolicy Bypass -File tools\e2e.ps1
```

`builtin_profiles_parse` checks that built-in profiles load: unique `id`s, valid hotkeys, values, `select` options, `slider` limits, signatures and `inject` templates. `e2e.ps1` checks the cheat engine against the test game.

---

## Русский

Как добавить игру или улучшить профиль. Профили лежат в `games/*.json` и встраиваются в программу при сборке. Чтобы проверить профиль без пересборки, положите файл в `%APPDATA%\FahrenHack\games\`: он заменит встроенный профиль с тем же `id`.

### Правила для профилей

1. **Только одиночные игры и режимы.** Не принимаются профили для онлайн-режимов, игр с античитом, рейтингов и соревнований.
2. **Без платного контента.** Никаких читов на DLC, микротранзакции, валюту за реальные деньги и разблокировку лицензий.
3. **Без обхода защиты.** Патчи (`patch`, `inject`) не должны отключать DRM, anti-tamper, проверки целостности или античит. Файлы игры на диске не меняются никогда.
4. **Только свои находки.** Не копируйте таблицы Cheat Engine, скрипты и офсеты из чужих трейнеров и форумов; ищите значения сканером FahrenHack. Если находка чужая, получите разрешение автора и укажите его.
5. **Без ассетов игры.** Никаких логотипов, обложек, скриншотов и текстов из игры. Описания читов — своими словами.

### Формат профиля

Формат тот же, что в английском разделе выше. Раздел `categories` необязателен: он задаёт порядок и иконки категорий, остальные категории идут следом в порядке появления.

### Поля чита

- `kind` — элемент управления:

  | `kind` | В интерфейсе | Что делает |
  |---|---|---|
  | `freeze` | переключатель | держит значение или патч кода, пока включён |
  | `set` | кнопка «Применить» | записывает `value` один раз |
  | `number` | «−», поле, «+» | показывает значение из игры, меняет его на `step`, пишет введённое; `min` и `max` ограничивают |
  | `slider` | ползунок | настройка от `min` до `max` с шагом `step`; `off_value` выключает чит. Для множителей, урона, скорости |
  | `select` | список и «Применить» | записывает выбранный вариант из `options` (`{ "label": "…", "value": "…" }`) |

- `vtype`: `i32`, `i64`, `f32`, `f64` или `u8` (один байт, для флагов игры; только в профилях).
- `value`: значение по умолчанию. Для `freeze` значение `null` означает «держать то, что было при включении»; для `slider` и `select` — начальная позиция.
- `off_value`: для `slider` — значение «выключено»; для `freeze` — что записать при выключении (флаг игры возвращается в 0, скорость в 1).
- `hotkey`: клавиша вроде `F1`, `CTRL+F2`, `SHIFT+NUM5`, `ALT+K`. Для `number` и `slider` это «больше», для `select` — следующий вариант.
- `hotkey_down`: «меньше» или предыдущий вариант (только `number`, `slider`, `select`).
- `info`: подсказка за значком «i». `warn`: предупреждение за жёлтым «!» о побочных эффектах.
- `icon`: тот же список, что в английском разделе.

### Способы найти значение (`locator`)

| `type` | Что это |
|---|---|
| `calibrate` | Мастер находит значение на ПК игрока: `mode: exact` для видимого числа, `unknown` для полоски. Затем строятся цепочки указателей. |
| `pointer` | Фиксированная цепочка `[[game.exe+0x1234]+0x10]+0x8`. |
| `aob` | Байтовая сигнатура в модуле; необязательный `rip` для RIP-адресации, затем `offsets`. |
| `patch` | Замена байтов по сигнатуре; при выключении исходные байты возвращаются. |
| `inject` | Инъекция кода: `size` байт (минимум 5) по сигнатуре заменяются переходом в код-пещеру, где выполняется `code`; при выключении исходные байты возвращаются. |
| `item` | Предмет в списке инвентаря: `pattern`, `rip`, `offsets` ведут (как в `aob`) к управляемому `List<T>`; FahrenHack ищет элемент, у которого объект предмета (по `item`, по умолчанию `10`) содержит u32 `id` по `id_at` (по умолчанию `28`), и берёт значение по `field`. |

### Шаблон `code` для `inject`

Байты в hex через пробел и подстановки: `{orig}` (затёртые байты; нельзя, если в них адресация относительно RIP), `{return}` (переход обратно, 5 байт), `{param}` (rel32 до ячейки параметра для инструкции, где смещение — последнее поле), `{param_abs}` (8-байтовый адрес параметра), `{addr:ID}` (rel32 до ячейки с текущим адресом значения другого чита этого профиля или 0, пока он неизвестен; FahrenHack держит его актуальным).

Параметр — значение ползунка, выбранный вариант или `value` переключателя в формате `vtype`. Ползунок меняет его на лету. Два чита не могут патчить один и тот же код одновременно. Пример — в английском разделе выше.

### Проверенные цепочки (`builds`)

Цепочки настроенного чита хранятся в `%APPDATA%\FahrenHack\calibration\<game>\<build>.json`. В профиль их можно добавить, если они **подтверждены минимум в двух запусках игры** (`confirmations` ≥ 2). Тогда чит на этой сборке работает сразу, без настройки.

### Перед pull request

```bash
cargo test
powershell -ExecutionPolicy Bypass -File tools\e2e.ps1
```

`builtin_profiles_parse` проверяет встроенные профили: уникальные `id`, клавиши, значения, варианты `select`, пределы `slider`, сигнатуры и шаблоны `inject`. `e2e.ps1` проверяет движок на тестовой игре.
