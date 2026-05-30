# @atomgit.com/hydra

[![npm version](https://img.shields.io/npm/v/@atomgit.com/hydra)](https://www.npmjs.com/package/@atomgit.com/hydra)
[![license](https://img.shields.io/npm/l/@atomgit.com/hydra)](https://atomgit.com/atomgit_atomcode/hydra)

**Hydra** — 开源终端 AI 编码助手。用自然语言描述任务，自动阅读代码、编辑文件、执行命令、验证结果。

## 安装

```bash
npm install -g @atomgit.com/hydra
```

安装完成后即可使用：

```bash
hydra
```

> 安装时 npm 会自动下载匹配当前平台的预编译二进制（darwin/linux arm64+x64, windows x64, ohos arm64）。

## 使用

```bash
# 交互模式（TUI）
hydra

# 指定项目目录
hydra -C /path/to/project

# 指定模型
hydra --model gpt-4o

# 非交互模式（headless）
hydra -p "解释这个仓库的架构"

# 继续上次对话
hydra --continue
```

## 卸载

```bash
npm uninstall -g @atomgit.com/hydra

# 或使用内置卸载命令（会保留配置文件）
hydra uninstall
```

## 版本对应

npm 版本号与 Hydra 发布版本一致。详见 [Releases](https://atomgit.com/atomgit_atomcode/hydra/releases)。

## 链接

- [源码仓库](https://atomgit.com/atomgit_atomcode/hydra)
- [Issues](https://atomgit.com/atomgit_atomcode/hydra/issues)
- [许可证](https://atomgit.com/atomgit_atomcode/hydra/blob/main/LICENSE)

---

Built with Rust, ratatui, and a lot of late nights.
