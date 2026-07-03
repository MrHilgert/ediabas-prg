# План: Настройки/конфиг (popup + .ini), сохранение темы, файловый i18n, выбор COM-порта

> Идеи пользователя, зафиксированные для реализации:
> 1. Отдельное **popup-окно настроек**; перенести туда выбор языка; настройки хранятся в **`.ini`**.
> 2. **Тема** (светлая/тёмная) сохраняется в настройки и подгружается при каждом запуске.
> 3. **Переводы (i18n)** хранятся в отдельных файлах — под каждый язык свой файл.
> 4. В настройках — **выбор COM-порта**, кроссплатформенно (**Windows и Linux**).

## Context (зачем)

Сейчас язык (в шапке `seg2 RU/EN`) и тема (в шапке ☀/☾) при каждом запуске **сбрасываются в дефолты** (`Ru`, `Dark`) — ничего не сохраняется. Строки интерфейса зашиты в код (`lang.rs::dict()`); файловая система переводов `tr()`/`locale/<lang>.txt` уже есть, но **файлов нет** → работает как passthrough. COM-порт захардкожен `"COM3"`. Цель: постоянные настройки в `.ini`, popup-окно настроек (язык + тема + порт), тема — из файла при старте, все переводы — в файлах.

## Текущее состояние (из разведки)

- **Нет слоя персистентности.** `App::new` (`gui/src/app.rs:94`) хардкодит `theme=Dark`, `lang=Ru`; тема применяется каждый кадр в `update()` (`ctx.set_visuals(theme::visuals(self.theme))`); `cc.storage` не читается; `main.rs` не использует персистентность eframe.
- **Нет крейтов** serde/ini/toml (минимализм зависимостей — парсеры ручные: `i18n` — `key<TAB>value`, `inpa` — CP1251). → `.ini` парсить/писать **вручную**, без новых зависимостей.
- **i18n двухуровневый:**
  - Tier 1 (chrome): `lang.rs::dict(lang) -> Dict` (~60 полей `&'static str`, RU/EN по `match`; много полей уже мертвы после чистки UI). Плюс похожие `match`-хелперы: `ecu.rs::Category::label`/`Module::name`, `data.rs::series_name`/`body_name`/`name`.
  - Tier 2 (динамика): `i18n.rs::tr(s, lang)` поверх `locale/ru.txt`/`locale/en.txt` (файлов **нет** → passthrough). `i18n::resolve(rel)` — общий walk-up резолвер (cwd + папка exe), уже переиспользуется для `SGDAT/*.ipo`.
- **Шапка** (`screens/mod.rs`): язык — `seg2(ui,c,"RU","EN",…)` (стр. 166); тема — `theme_toggle` (☀/☾); чип связи использует `dict(app.lang).interface`.
- **`Lang`** — `enum { Ru, En }` (`lang.rs:5`); **`Theme`** — `enum { Dark, Light }` + `toggled()` (`theme.rs:9`). Ни у одного нет `Default`/serde.
- **COM-порт:** worker уже принимает порт параметром — `Cmd::Connect { port: String, baud: u32, prg: String }` (`worker.rs:17`), обрабатывается в `worker.rs:83` → `connect(&port, baud, &prg)` (`worker.rs:167`). Порт **захардкожен `"COM3"` только в месте отправки `Cmd::Connect`** (уточнить актуальный сайт диспатча). Ошибки `err_port_busy`/`err_no_adapter` содержат «COM3» текстом (`lang.rs:116-117,169-170`).
- **Энумерация портов:** крейт `serialport` (v4) — зависимость **lib `ediabas-prg`** (`src/driver/serial.rs`, `src/config.rs`), у него есть кроссплатформенный `serialport::available_ports()` (Windows → `COMx`, Linux → `/dev/ttyUSB*`,`/dev/ttyS*`). GUI **напрямую от serialport не зависит** (`gui/Cargo.toml`: eframe/egui/ediabas/inpa) → перечисление портов лучше отдать тонкой функцией из lib.

## Дизайн

### Часть 1 — Модуль конфигурации (`gui/src/config.rs`, новый)
- `pub struct Settings { pub lang: Lang, pub theme: Theme, pub port: Option<String> }` + `impl Default` → (`Ru`, `Dark`, `None` = автовыбор/первый доступный). (`baud` пока не выносим — остаётся 9600; см. открытые решения.)
- **Ручной `.ini`** (без новых зависимостей). Формат:
  ```ini
  # eDIAG settings
  language = ru        # ru | en
  theme    = dark      # dark | light
  port     = COM3      # COM3 (Windows) | /dev/ttyUSB0 (Linux); пусто = авто
  ```
  Парсер: пропускать пустые/`#`/`[section]`; делить по первому `=`; тримить; матчить ключи. Сериализация обратно — тем же простым форматом.
- `config_path() -> PathBuf` — папка exe / `settings.ini` (фолбэк cwd). Один путь для чтения и записи. **[РЕШЕНИЕ: рядом с exe, портативно; альтернатива — `%APPDATA%\eDIAG\` для per-user.]**
- `Settings::load() -> Settings` — прочитать `config_path()` (фолбэк `i18n::resolve("settings.ini")`); распарсить; при отсутствии/ошибке → `Settings::default()`.
- `Settings::save(&self)` — записать `config_path()`; ошибки записи глотать (best-effort).
- `Lang`/`Theme` ↔ строки вручную: `"ru"/"en"`, `"dark"/"light"` (serde не нужен).

### Часть 2 — Прокинуть персистентность в App
- `App::new`: вместо хардкода — `let s = Settings::load();` → `theme: s.theme, lang: s.lang, port: s.port`; применить `theme::visuals(s.theme)` как сейчас.
- **Сохранение по изменению** (одно место): добавить в `App` поле `saved: (Lang, Theme, Option<String>)`; в конце `update()` — если текущее `(lang, theme, port)` отличается, вызвать `Settings{…}.save()` и обновить `saved`.

### Часть 3 — Popup настроек (`gui/src/screens/settings.rs`, новый — или в `screens/mod.rs`)
- Модалка по образцу `connect_modal` (`ecu_select.rs:353`): затемняющий `egui::Area` (Order::Middle, глотает клики) + центрированное `egui::Window` (без title_bar, anchor CENTER_CENTER, panel-frame).
- В `App` — `pub show_settings: bool`.
- Содержимое (переиспользовать `seg2`):
  - Заголовок «Настройки» (через `tr`).
  - **Язык**: `seg2(ui,c,"RU","EN",lang==Ru)` → `app.lang`. (Перенесён из шапки.)
  - **Тема**: `seg2("Тёмная","Светлая",…)` или `theme_toggle` → `app.theme`. (Зеркало.)
  - **COM-порт**: выпадающий список/строка портов из `ediabas::available_ports()` (см. Часть 5) + кнопка «Обновить» + вариант «Авто». Выбор пишется в `Settings.port`. Работает на Windows (`COMx`) и Linux (`/dev/tty*`).
  - Кнопка «Закрыть».
- **Триггер**: в шапке — **кнопка-шестерёнка** (рисованная, в стиле `theme_toggle`) → `show_settings = true`. Убрать из шапки `seg2 RU/EN`.
- Рендерить модалку централизованно в конце `App::update()` (`screens::settings_modal(self, ctx)`), чтобы перекрывала любой экран.
- **[РЕШЕНИЕ по теме: тумблер ☀/☾ остаётся в шапке И дублируется в настройках; оба пишут в `.ini`.]**

### Часть 4 — Файловый i18n (`locale/ru.txt`, `locale/en.txt`, новые)
**A. Chrome → tr() (обязательно):**
- Создать `locale/en.txt`: для каждой живой русской строки chrome → английская (из EN-ветки `dict()`). `locale/ru.txt` — опционально/identity.
- Заменить живые `dict(app.lang).<field>` (в `screens/mod.rs`, `chassis_select.rs`, `ecu_select.rs`, `app.rs`) на `tr("<русский>", app.lang)`.
- Выкинуть мёртвые поля `Dict` (`title_sub`, `connected`, `adapter`, `scan_all`, `lg2_*`, `col_*`, `st_*`, `nolink_note`, `dtc`, `part_no`, `version`, `live_data`, `read_faults`, `clear_mem`, …), затем удалить `struct Dict` + `dict()`; оставить `enum Lang`.

**B. [Расширение, опционально] Данные-имена → tr() (`ecu.rs`/`data.rs`):**
- Прогнать `Category::label`, `Module::name`, `data.rs::series_name`/`body_name`/`name` через `tr()`; добавить пары в locale-файлы. Коды кузовов/двигателей и годы — **не переводить**.

**[РЕШЕНИЕ по объёму: A сейчас; B — следующий шаг. «Полностью» = A+B.]**

- Засеять `locale/en.txt`: `КУЗОВ→CHASSIS`, `ВЫБОР ЭБУ→SELECT ECU`, `ПОДКЛЮЧИТЬСЯ→CONNECT`, `ИНТЕРФЕЙС→INTERFACE`, `Настройки→Settings`, `Нестабильно→Unstable`, `Инициализация…→Initializing…` и т.д. + уже `tr()`-нутые строки сессии.

### Часть 5 — Выбор COM-порта (кроссплатформенно)
- **Энумерация (в lib `ediabas-prg`):** `pub fn available_ports() -> Vec<String>` (в `src/driver/mod.rs` или `serial.rs`), оборачивающая `serialport::available_ports()` → `port_name`, отсортировать. Кроссплатформенно: Windows → `COM1..COMn`, Linux → `/dev/ttyUSB*`,`/dev/ttyACM*`,`/dev/ttyS*`. Ошибку → пустой `Vec`. Реэкспорт из `lib.rs` (`pub use driver::available_ports;`) → GUI зовёт `ediabas::available_ports()` без прямой зависимости от serialport.
- **Прокинуть выбранный порт в connect:** в месте отправки `Cmd::Connect` (сейчас `"COM3"`) брать порт из настроек: `app.port` (или первый из `available_ports()`, если `None`). Baud пока `9600`.
- **«Авто»:** если `Settings.port == None` — первый доступный порт; если портов нет — прежнее поведение (Session::open вернёт понятную ошибку). Показать активный порт в popup.
- **Тексты ошибок:** заменить хардкод «COM3» в `err_port_busy`/`err_no_adapter` на подстановку активного порта (уходят в файловый i18n, Часть 4).

## Файлы

- **Новые:** `gui/src/config.rs`; `gui/src/screens/settings.rs` (или блок в `screens/mod.rs`); `locale/ru.txt`; `locale/en.txt`.
- **Правки:** `gui/src/main.rs` (`mod config`); `gui/src/app.rs` (загрузка настроек в `new`, `show_settings`, `port`, save-on-change + поле `saved`); `gui/src/screens/mod.rs` (убрать `seg2 RU/EN`, добавить шестерёнку + модалку, чип через `tr`); `gui/src/lang.rs` (удалить `Dict`/`dict`, оставить `Lang`); `gui/src/screens/chassis_select.rs` + `ecu_select.rs` (`dict`→`tr`); место отправки `Cmd::Connect` (порт из настроек); `src/driver/mod.rs`(или `serial.rs`) + `src/lib.rs` (новый `available_ports()` + реэкспорт); `.gitignore` (возможно, игнорить `settings.ini`).
- **Переиспользовать:** `i18n::tr`, `i18n::resolve`, `seg2` (`screens/mod.rs:83`), `theme_toggle`, `theme::visuals`, паттерн `connect_modal` (`ecu_select.rs:353`), `serialport::available_ports()`.

## Verification

- `cargo run -p ediag`: сменить язык в popup и тему → закрыть → перезапустить → и язык, и тема восстановлены из `settings.ini`. Проверить файл рядом с exe (в dev — `target/debug/settings.ini`) — читаемый.
- Удалить `settings.ini` → перезапуск → дефолты (`Ru`,`Dark`), файл пересоздаётся при первом изменении.
- Отредактировать `locale/en.txt`, переключиться на EN → интерфейс берёт значения из файла; удалить ключ → passthrough (русский). Новый язык — файлом без пересборки.
- **COM-порт (обе ОС):** список портов не пуст (Windows `COMx`, Linux `/dev/ttyUSB*`); выбрать → CONNECT идёт на него; выбор сохраняется и восстанавливается; «Обновить» перечитывает после втыкания адаптера; «Авто» берёт первый.
- `cargo build -p ediag-gui` чисто; тесты `inpa` не затронуты; `cargo build` (lib) с `available_ports()` чист.

## Открытые решения

- **Место конфига:** рядом с exe (портативно). Альтернатива — `%APPDATA%\eDIAG\` (per-user).
- **Объём i18n:** A (chrome) сейчас; B (имена ЭБУ/кузовов) — follow-up.
- **Тема в UI:** в шапке + зеркало в настройках.
- **COM-порт:** в плане (Часть 5), кроссплатформенно. **baud (9600) и протокол (DS2)** пока захардкожены — вынос в тот же popup естественный следующий шаг.
