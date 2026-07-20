# herdr-workbench

本地 Herdr 插件开发工作台。每个子目录一个独立插件（各自有 `herdr-plugin.toml`），用 `herdr plugin link` 挂到本机 Herdr 上迭代。

- Herdr: `0.7.4+`（插件 API 要求 `min_herdr_version >= 0.7.0`）
- 文档: [Plugins](https://herdr.dev/docs/plugins/) · [CLI](https://herdr.dev/docs/cli-reference/) · [Socket API](https://herdr.dev/docs/socket-api/)
- 官方示例: [ogulcancelik/herdr-plugin-examples](https://github.com/ogulcancelik/herdr-plugin-examples)

## 目录

```text
herdr-workbench/
  plugins/
    starter/          # 最小可运行插件（Bash）
  README.md
```

## 快速开始

```bash
# 1. 确认 Herdr 在跑
herdr status

# 2. 链接 starter 插件
herdr plugin link ./plugins/starter

# 3. 查看与调用
herdr plugin list
herdr plugin action list --plugin workbench.starter
herdr plugin action invoke workbench.starter.hello
herdr plugin config-dir workbench.starter
herdr plugin log list --plugin workbench.starter
```

开发时改完脚本直接再 `invoke` 即可；`plugin link` 不会跑 `[[build]]`，有构建步骤时在插件目录里自己先 build。

## 常用命令

```bash
herdr plugin link ./plugins/<name>     # 注册本地插件
herdr plugin unlink <plugin_id>        # 仅注销，不删文件
herdr plugin enable|disable <id>
herdr plugin uninstall <id>            # GitHub 安装的还会删 checkout
herdr plugin pane open --plugin <id> --entrypoint <pane_id>
```

## 新建插件

1. 复制 `plugins/starter` 为新目录
2. 改 `herdr-plugin.toml` 里的 `id` / `name` / `version` / actions
3. `herdr plugin link ./plugins/<name>`
4. 需要持久配置/状态时用：
   - `HERDR_PLUGIN_CONFIG_DIR`（用户可编辑配置，如 `.env`）
   - `HERDR_PLUGIN_STATE_DIR`（运行时状态）
   - **不要**把凭据写进插件源码根目录

回调 Herdr 时优先用 `HERDR_BIN_PATH`（跨平台），需要原始请求再用 `HERDR_SOCKET_PATH`。

## 发布

1. 把插件放到独立公开 GitHub 仓库（或 monorepo 子目录）
2. 根目录（或子目录）放好 `herdr-plugin.toml`
3. 仓库加 topic：`herdr-plugin`（marketplace 约 30 分钟收录）
4. 安装：`herdr plugin install owner/repo[/subdir]`
