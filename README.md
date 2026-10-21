# corral

VS Code 风格左侧边栏（Rust）— Herdr 插件，也可独立运行。

![screenshot](screenshot.png)

## 安装

运行时（`corral`、`corral-github`）和 Herdr 插件分别安装。
使用插件需要先安装 Herdr 0.7.0 或更新版本。

### Fedora

安装下载工具和默认功能使用的依赖：

```bash
sudo dnf install curl tar xz coreutils git bash jq gh neovim git-delta glib2 imv
```

在仓库目录运行二进制安装脚本：

```bash
bash scripts/install-fedora.sh
export PATH="$HOME/.local/bin:$PATH"
```

脚本自动选择 `x86_64` 或 `aarch64` 的最新 GitHub Release，下载后校验
SHA-256，并安装以下文件：

- `~/.local/bin/corral`
- `~/.local/bin/corral-github`
- `~/.local/share/herdr-corral/config.default.sh`

安装到当前用户目录，无需管理员权限。把上面的 `export PATH` 加入
`~/.zshrc`（Bash 使用 `~/.bashrc`），让新终端能够找到程序。
脚本结束时会输出带有准确版本标签的 `herdr plugin install ... --ref ...`
命令；执行该命令注册插件，再打开左侧 32 列 Corral：

```bash
herdr plugin action invoke corral.open
```

升级时重新运行同一脚本。也可指定目标版本进行安装、升级或回退：

```bash
bash scripts/install-fedora.sh v0.1.9
herdr plugin install xifan2333/herdr-corral --ref v0.1.9
```

将示例中的 `v0.1.9` 替换为目标版本，使运行时和插件使用同一版本。
升级后重新打开 Corral；已有的 `~/.config/corral/config.sh` 会保留，
需要的新默认项可对照安装后的配置模板合并。
使用 GitHub 功能前，运行 `gh auth login` 完成登录。

脚本安装所选 Release 的发布二进制；本地源码改动按下方“开发”小节构建。

### Arch Linux

```bash
yay -S herdr-corral-bin                              # 只装二进制
herdr plugin install xifan2333/herdr-corral          # 注册插件（不编译）
herdr plugin action invoke corral.open               # 打开左 dock
```

### 独立运行

直接运行二进制：

```bash
corral
corral-github issue --repo owner/repo 123
```

## 配置

所有快捷键定义在 `~/.config/corral/config.sh`（首次启动自动 seed，
模板见 [`config.default.sh`](config.default.sh)）。已有配置不会被自动迁移或覆盖；
如需恢复最新默认配置，删除该文件后重新启动 Corral。

语法：`corral bind <key> <action>`

改键后下次启动 Corral 生效，无需重编译。

依赖 [GitHub CLI](https://cli.github.com/) `gh`；图片用默认 `imv` 打开，
可通过 `CORRAL_GITHUB_IMAGE_VIEWER` 覆盖。

### 跟随 shell 目录

Herdr 布局可以把一个 shell 与 Corral 侧栏绑定。shell 执行 `cd` 后，
Corral 的文件树、SCM 和 GitHub 视图会一起切换目录；编辑器焦点变化保持该绑定。

使用 `herdr pane list` 查看窗格 ID，再为 shell 上报绑定元数据。
以下示例中 `w1:p1` 是 Corral，`w1:p3` 是 shell，替换为当前标签页的实际 ID：

```bash
herdr pane report-metadata w1:p3 --source corral-layout \
  --token corral-shell-owner=w1:p1
```

每个侧栏绑定一个 shell。更换绑定时，先在原 shell 窗格上执行
`herdr pane report-metadata <原窗格ID> --source corral-layout --clear-token corral-shell-owner`，
再上报新绑定。独立运行的 Corral 使用启动目录。

### Neovim Diff 浏览

在 Herdr 或 WezTerm 中，SCM diff 会打开到复用的 Neovim terminal buffer，
并停留在 Normal mode，方便直接搜索、复制和滚动。默认使用 `delta` 渲染；
可在 Neovim 配置中加入以下映射，用 `←` / `→` 跳到上一处 / 下一处变更，
并在打开后自动跳到第一处变更（`j` / `k` 仍用于逐行滚动）。完整可复制版本见
[`examples/nvim-corral.lua`](examples/nvim-corral.lua)：

```lua
-- Diff review in terminal buffers: ←/→ to jump across changes, auto-jump on open.
local pat = [[\v(^\s*\d+\s*⋮\s*│|^\s*⋮\s*\d+\s*│|^\s*\d+\s*[-+])]]
local grp = vim.api.nvim_create_augroup("CorralDiffReview", { clear = true })

vim.api.nvim_create_autocmd("TermOpen", {
  group = grp,
  callback = function(ev)
    vim.defer_fn(function()
      if not vim.b[ev.buf].corral_preview then return end
      vim.keymap.set("n", "<Right>", function() vim.fn.search(pat, "W") end,
        { buffer = ev.buf, desc = "Next diff change", silent = true })
      vim.keymap.set("n", "<Left>", function() vim.fn.search(pat, "bW") end,
        { buffer = ev.buf, desc = "Previous diff change", silent = true })
      vim.defer_fn(function()
        if vim.api.nvim_buf_is_valid(ev.buf) then pcall(vim.fn.search, pat, "W") end
      end, 120)
    end, 20)
  end,
})
```

例如可放在 `~/.config/nvim/lua/config/keymaps.lua` 或直接放入
`init.lua`；也可以复制参考文件到 `lua/config/corral.lua` 并在配置中调用
`require("config.corral")`。GitHub detail 属于交互式 TUI，会自动进入
terminal-input mode，不受以上 Normal-mode diff 浏览方式影响。

## 快捷键

### 全局

| 键 | 动作 |
|---|---|
| `1` `2` `3` | Explorer / SCM / GitHub |
| `q` `Ctrl+c` | 退出 |
| `j` `k` `↓` `↑` | 上下 |
| `g` `G` | 顶 / 底 |
| `PgUp` `PgDn` | 翻页 |
| `h` `l` `←` `→` | 折叠 / 展开 |
| `z` | 全部折叠 |
| `Enter` | 切换（文件暂存 / 打开） |
| `r` | 刷新 |

### Explorer

| 键 | 动作 |
|---|---|
| `.` | 显示/隐藏点文件 |
| `a` `d` `r` | 新建 / 删除到回收站 / 重命名 |

### SCM

| 键 | 动作 |
|---|---|
| `s` `Space` | 暂存 / 取消暂存 |
| `a` `u` | 全部暂存 / 取消暂存 |
| `o` | 查看 diff |
| `c` | 聚焦 commit message |
| `A` | 生成 commit subject |
| `D` | 丢弃改动 |
| `S` | 同步 |
| `Esc` | 取消 |

### GitHub 侧栏

| 键 | 动作 |
|---|---|
| `i` `p` `a` `w` | Issues / PRs / Actions / Workflows |
| `Enter` `o` | 打开 detail |
| `d` `c` `x` `L` | Diff / Checks / 失败日志 / 日志 |
| `f` | 过滤 |
| `]` | 加载更多 |
| `s` | 切换状态 |
| `t` | workflow dispatch |

### GitHub Detail

| 键 | 动作 |
|---|---|
| `c` | 回复 |
| `a` | Approve |
| `x` | context action |
| `D` | Close / Reopen |
| `m` | Merge（选策略） |
| `o` | 打开图片 |
| `R` `A` | Rerun failed / all |
| `Enter` `Esc` `y` | 确认 / 取消 |

## 模块

| 模块 | 作用 |
|---|---|
| `app` | sidebar 事件循环 |
| `config` | shell 配置引擎 |
| `feature/{explorer,scm,github}` | sidebar FeatureView |
| `github/{gh,model,detail}` | `gh` CLI + 数据 + 全宽客户端 |
| `git` | status 发现、stage / unstage |
| `ui` | palette / icons / activity |
| `herdr` | 宿主 CLI |

## 开发

```bash
cargo build --release
export PATH="$PWD/target/release:$PATH"
herdr plugin link .
herdr plugin action invoke corral.open
```

提交前运行与 CI 相同的检查：

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked
cargo test --all-targets --locked
cargo test --doc --locked
bash -n config.default.sh scripts/*.sh
shellcheck config.default.sh scripts/*.sh
```
