# corral

VS Code 风格的**左侧边栏**（Rust）。**开发中**。

对齐 [herdr-sidebar](https://github.com/alexarthurs/herdr-sidebar) 的形状，而不是整页 workbench：

```text
┌────────────┬──────────────────────────────┐
│  Sidebar   │  用户原有 panes / 未来 Preview │
│  [][][] │                              │
│  树 / 列表  │                              │
└────────────┴──────────────────────────────┘
```

- **一个** left-docked Herdr pane（重复打开只聚焦现有 pane）
- 启动时选择真正最左 pane，按约 32 列计算 split ratio
- 顶栏横排 feature icons：Explorer / SCM / GitHub（进程内切换）
- 详情（文件 / diff / PR）→ **以后**用独立 preview pane + control file
- 也可 standalone：`./target/release/corral`

## 开发

```bash
cargo build --release

# 独立
./target/release/corral

# 插件（左 dock）
herdr plugin link .
herdr plugin action invoke corral.open
```

所有快捷键均由可编辑的 `config.sh` 通过 `corral bind <key> <action>`
注入，程序内不保留默认键位分支。默认配置中：`1`/`2`/`3` 切 feature，
`j`/`k` 导航，`q` 退出。

Explorer：`h`/`l` 折叠/展开，`PageUp`/`PageDown` 翻页，`.` 显示/隐藏
点文件（始终排除 `.git`），`z` 折叠全部，`Enter` 打开。

SCM：`Enter`/`s` 暂存或取消暂存，`a` 全部暂存，`u` 全部取消暂存，
`o` 查看 staged/worktree/untracked diff，`c` 聚焦内联提交信息并用 `Enter`
提交，`D` 确认后丢弃改动，`S` 后台同步，`h`/`l` 折叠/展开分组，`r` 刷新；
保持打开时每 1.5 秒自动刷新。所有键位仍只来自 `config.sh`。

独立 diff 过滤器（与 Corral 使用同一主题）：

```bash
git diff | ./target/release/corral-diff | less -R
```

SCM 默认使用 `corral-diff`；可在 `config.sh` 设置
`CORRAL_DIFF_TOOL=corral|difft|fancy|delta|git`。

## 模块

| 模块 | 作用 |
|---|---|
| `host` | plugin / standalone 上下文 |
| `theme` | Herdr 主题色表 |
| `icons` | Nerd Font 检测 |
| `feature` | Explorer / SCM / GitHub |
| `git` | 仓库发现、status 解析、stage / unstage |
| `diffview` / `corral-diff` | 独立主题化 diff 过滤器 |
| `layout` | activity + body 几何 |
| `app` | sidebar 事件循环 |

## 下一步

1. SCM discard 确认、commit 反馈、后台 sync
2. 自动 ensure + preview ctl 协议
3. GitHub 独立 adapter + view
