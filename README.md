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
- 文件、SCM diff 和 GitHub 长内容复用一个 owner-scoped nvim pane，通过 RPC 切换 buffer
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

## 发布 / Arch

打 `v*` tag 会跑 `.github/workflows/release.yml`，上传：

- `herdr-corral-<ver>-x86_64.tar.xz`
- `herdr-corral-<ver>-aarch64.tar.xz`

手动安装 tarball：

```bash
tar -xJf herdr-corral-<ver>-x86_64.tar.xz
cd herdr-corral-<ver>-x86_64
./install.sh   # PATH 含 ~/.local/bin → ~/.local，否则 /usr
```

AUR 包在 [aur-auto](https://github.com/xifan2333/aur-auto) 的 `herdr-corral-bin`：

```bash
yay -S herdr-corral-bin
```

所有快捷键均由可编辑的 `config.sh` 通过 `corral bind <key> <action>`
注入，程序内不保留默认键位分支。配置使用 `CORRAL_CONFIG_VERSION`
标记版本，供后续非破坏式迁移。默认配置中：`1`/`2`/`3` 切 feature，
`j`/`k` 导航，`q` 退出。

Explorer：`h`/`l` 折叠/展开，`PageUp`/`PageDown` 翻页，`.` 显示/隐藏
点文件（始终排除 `.git`），`z` 折叠全部，`Enter` 打开；`a` 新建（名称
以 `/` 结尾时创建目录），`d` 确认删除，`r` 重命名。根目录使用 recursive
filesystem watcher，Corral 内外的文件变化都会自动更新，无需手动刷新。

SCM：`Enter`/`s` 暂存或取消暂存，`a` 全部暂存，`u` 全部取消暂存，
`o` 查看 staged/worktree/untracked diff，`c` 聚焦内联提交信息并用 `Enter`
提交，`A` 异步生成智能 commit message，`D` 确认后丢弃改动，`S` 后台同步，
`h`/`l` 折叠/展开分组，`r` 刷新；Graph、Commits、File History、Branches、
Worktrees、Remotes、Stashes、Tags
均为真实 drawer。保持打开时每 1.5 秒自动刷新。所有键位仍只来自 `config.sh`。

独立 diff 过滤器（与 Corral 使用同一主题）：

```bash
git diff | ./target/release/corral-diff | less -R
```

SCM 默认使用 `corral-diff`；可在 `config.sh` 设置
`CORRAL_DIFF_TOOL=corral|difft|fancy|delta|git`。

智能 commit provider 完全由 `config.sh` 控制。默认使用非交互 `pi -p`；
也可将 `CORRAL_COMMIT_SUGGEST_CMD` 替换为其他 agent 或自定义脚本。命令
接收包含 label 规则、文件列表和 diff 的消息参数，并从 stdout 返回一行 subject。

GitHub：依赖已登录的 [GitHub CLI](https://cli.github.com/) `gh`，不在 Corral
内保存 token。Issues、Pull Requests、Actions 与 SCM 一样是可选择、可折叠的
状态树 section，`i`/`p`/`a` 定位 section，`Enter` 或 `h`/`l` 折叠展开；`f`
本地过滤，`]` 加载更多，`s` 循环 open/closed/merged/all 状态。

`Enter` 在共享 nvim pane 启动独立的 `corral-github`：Issue 提供正文、comments、
回复和 Close/Reopen；PR 提供 Conversation、Files、Diff、Checks、回复、Approve、
Request Changes、merge method picker（merge/squash/rebase + head SHA 确认）和 Close/Reopen；
Workflows 是独立 section，列出 active workflow，支持 `workflow_dispatch` 参数表单；
Actions 提供 Summary、Jobs、logs、Cancel、Rerun Failed/All。Issue/PR 正文中的图片
渲染为 `[image] ...` 文本链接：鼠标点击或按 `o` 下载到缓存并用 `imv` 打开
（`CORRAL_GITHUB_IMAGE_VIEWER` 可覆盖）。Workflow Dispatch 使用
`t`/`Enter`，有 inputs 时先填表再确认，无 inputs 时直接确认，触发到默认分支。所有
网络和 mutation 均在后台运行，正文通过 stdin 传给 `gh`，破坏性操作必须确认。
GitHub Enterprise 使用
`[HOST/]OWNER/REPO` selector。`corral-github` 也可独立运行：

```bash
corral-github issue --repo owner/repo 123
corral-github pr --repo owner/repo 456 --view diff
corral-github run --repo owner/repo 789 --view jobs
```

## 模块

| 模块 | 作用 |
|---|---|
| `host` | plugin / standalone 上下文 |
| `theme` | Herdr 主题色表 |
| `icons` | Nerd Font 检测 |
| `feature` | Explorer / SCM / GitHub |
| `git` | 仓库发现、status 解析、stage / unstage |
| `github` | `gh` adapter、仓库身份和 typed models |
| `github::detail` / `corral-github` | Issue / PR / Actions 全宽交互客户端 |
| `diffview` / `corral-diff` | 独立主题化 diff 过滤器 |
| `layout` | activity + body 几何 |
| `app` | sidebar 事件循环 |

## 下一步

1. GitHub review-thread 回复与 choice/boolean input 更完整编辑
2. GitHub Actions artifacts 下载
3. editor pane/socket orphan 回收
4. Explorer Change Folder
5. SCM drawer destructive-action confirmation与自动 ensure / toggle
