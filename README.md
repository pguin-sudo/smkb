# smkb

Передача мыши и клавиатуры между двумя компьютерами с Linux и Hyprland по UDP.
`smkb-master` захватывает устройства ввода, `smkb-slave` воспроизводит события
через uinput. Соединение использует общий ключ PSK и шифрование.

Структура workspace:

- `smkb-app` — общие аргументы запуска, загрузка YAML и PSK, сигналы завершения.
- `smkb-master` — захват ввода, переключение по краю монитора и контроль соединения.
- `smkb-slave` — поиск master, переподключение и воспроизведение событий.
- `smkb-platform` — evdev, uinput и Hyprland IPC.
- `smkb-proto` — handshake, формат пакетов и надёжная доставка событий.

Сборка и проверки:

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Примеры конфигурации находятся в `examples/`. По умолчанию приложения читают
`$XDG_CONFIG_HOME/smkb/master.yaml` и `$XDG_CONFIG_HOME/smkb/slave.yaml`;
если `XDG_CONFIG_HOME` не задан, используется `$HOME/.config`.
Путь можно передать через `--config PATH`, справка доступна через `--help`.

Оба приложения должны использовать одинаковый PSK: файл из 32 байт либо
64 шестнадцатеричных символов. Для работы нужны доступ к устройствам ввода
(master), `/dev/uinput` (slave) и запущенный Hyprland.
