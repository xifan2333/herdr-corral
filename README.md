# corral

VS Code 风格的 Herdr 终端工作台插件（Rust）。**开发中**。

单插件仓库：`herdr-plugin.toml` 在根目录，`herdr plugin link .` 挂到本机 Herdr。

- Herdr: `0.7.0+`
- 文档: [Plugins](https://herdr.dev/docs/plugins/) · [CLI](https://herdr.dev/docs/cli-reference/) · [Socket API](https://herdr.dev/docs/socket-api/)

## 目标

把 Explorer / Source Control / GitHub 收进可开关的侧边 pane，做成 VS Code 风格的 activity bar。（尚未实现）

## 已实现：`corral::theme` 配色模块

只做一件事：**找到 Herdr 当前主题的色表并暴露**，供后续组件共用。

Herdr 不通过 CLI/socket 暴露解析后的主题色，所以本模块**把 Herdr 源码里的主题表原样搬过来**（逐字节 port 自 `herdr/src/app/state.rs` + `config/theme.rs`，v0.7.4），并和 Herdr 一样地解析：

1. 用 `serde`/`toml` 读 Herdr `config.toml` 的 `[theme]`（`name` / `auto_switch` / `dark_name` / `light_name` / `[theme.custom]`）
2. 查内置命名主题（18 套：catppuccin / terminal / dracula / nord / …）
3. 应用 `[theme.custom]` 覆盖

零探针、无需 TTY、完全确定，且和 Herdr 逐字节一致。

颜色类型就是 Herdr 用的那个——**`ratatui::style::Color`**。RGB 主题带 truecolor 值；`terminal` 主题的 token 是 ANSI 具名色（`Blue`/`Reset`/…），渲染时跟随终端调色板。序列化由 ratatui 自己的 serde 实现提供（`"#282A36"` / `"Blue"` / `"Reset"`）——**无任何手写拼串**。

```rust
use corral::theme::Palette;

let p = Palette::resolve();     // 读 Herdr config，无需 TTY
let accent = p.accent;          // ratatui::style::Color，任意 TUI 直接用
let red    = p.red;             // dracula: Rgb(255,85,85) / terminal: LightRed
```

输出（`name="terminal"`，token 为 ANSI 具名，故随终端）：

```json
{ "name": "terminal", "accent": "Blue", "panel_bg": "Reset", "red": "LightRed", "...": "..." }
```

换成 RGB 主题（如 dracula）则是真实 hex：`"panel_bg": "#282A36"`、`"red": "#FF5555"`。

## 开发

```bash
# 构建（plugin link 不跑 [[build]]，本地自己 build）
cargo build --release
cargo test --release

# 链接插件
herdr plugin link .
herdr plugin list

# 检查解析出的主题（无需 TTY）
./target/release/corral theme                 # 带色块的人读输出
./target/release/corral theme --json          # 机器可读
./target/release/corral theme --name dracula  # 预览指定内置主题
```

## 目录

```text
herdr-corral/
  herdr-plugin.toml    # 插件清单（根目录，含 [[build]]）
  Cargo.toml
  src/
    lib.rs             # crate corral：pub mod theme
    theme.rs           # 配色模块：Herdr 主题表 port + serde config 解析 + Palette (ratatui Color)
    main.rs            # dev CLI：corral theme
  README.md
  .gitignore           # 忽略 target/；提交 Cargo.lock
```

## 约定

- 回调 Herdr 优先用 `HERDR_BIN_PATH`（跨平台），要发原始 JSON 再用 `HERDR_SOCKET_PATH`
- 用户配置放 `HERDR_PLUGIN_CONFIG_DIR`，运行时状态放 `HERDR_PLUGIN_STATE_DIR`
- 不要把凭据/状态写进插件根目录（GitHub 安装时那是托管 checkout）
