<div align="center">

# cursor-byok

Bring your own model APIs to Cursor.

[Download](https://github.com/Accompany-cc/cursor-byok/releases) · [Report an Issue](https://github.com/Accompany-cc/cursor-byok/issues) · [中文说明](./README-CN.md)

[![Release](https://img.shields.io/github/v/release/Accompany-cc/cursor-byok?style=flat-square)](https://github.com/Accompany-cc/cursor-byok/releases)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](./LICENSE)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey?style=flat-square)

</div>

![Connect cursor-byok to a wide range of model APIs](./images/en-brand-1.png)

![cursor-byok dashboard](./images/en-home-1.png)

## About

cursor-byok is an open-source local model gateway for Cursor. Connect your own OpenAI- and Anthropic-compatible APIs while keeping Agent capabilities such as tool calling, Skills, MCP, and multi-turn conversations.

## 个人维护说明 / Personal maintenance

本 Fork 由 [Accompany-cc](https://github.com/Accompany-cc) 以个人自用为主维护，欢迎贡献，但优先自用需求，不承诺响应或合并时间。这不是原作者或社区维护者的官方发行版。完整中文说明见 [README-CN](./README-CN.md#个人维护说明--personal-fork)。

Based on [leookun/cursor-byok](https://github.com/leookun/cursor-byok), using the [leehainuo community baseline](https://github.com/leehainuo/cursor-byok/commit/e69bda0f6a8da4f002b934bc86c572d32d824800), with [PR 56](https://github.com/leehainuo/cursor-byok/pull/56), [PR 57](https://github.com/leehainuo/cursor-byok/pull/57), [PR 58](https://github.com/leehainuo/cursor-byok/pull/58), and local BidiAppend binary compatibility fixes. Original copyrights and MIT notices are retained; attribution does not imply endorsement.

**1.0.3 is candidate source code, not a Windows-validated release.** Existing 1.0.2 test executables are unchanged and are not rebuilt by this source update. See [validation boundaries](./docs/bidi-binary-compat.md) and [Windows test notes](./docs/windows-test-build.md).

Automatic update checks, downloads and installation are disabled until independent signing is configured and reviewed. Download future packages manually from this fork's Releases. Product name, application identifier (`dev.cursorbyok.desktop`) and configuration paths remain unchanged for migration: installing may overwrite an existing Cursor BYOK installation and reuse its settings. Quit the app and back up configuration (including API keys) first.

## Features

- **Your own models:** Configure API endpoints, keys, model IDs, and request parameters.
- **Model management:** Add, edit, reorder, and batch-test model configurations.
- **Connection benchmarks:** Measure response latency and generation speed.
- **Session metrics:** Track token usage, cache hits, and conversation activity.
- **Cross-platform:** Available for macOS, Windows, and Linux.

![cursor-byok model settings](./images/en-model-1.png)

## How It Works

```text
Cursor client
    │
    │ Agent requests and tool results
    ▼
cursor-byok local service
    │
    │ OpenAI- / Anthropic-compatible requests
    ▼
Your model API
```

cursor-byok handles protocol adaptation, model request forwarding, tool-call coordination, and conversation state on your machine. API keys and application settings are stored locally; requests are still sent to the model provider you configure.

## Why This Project

Many Agent products bundle their tool capabilities with a fixed set of models, subscriptions, and billing options, leaving users limited to the channels offered by the platform.

cursor-byok is built to return model choice to the user. Developers can make full use of the APIs and credits they already have, choose the models and providers that fit their needs, and self-host related services when required.

## Acknowledgements

Thanks to original author [leookun](https://github.com/leookun), community maintainer [leehainuo](https://github.com/leehainuo), and all PR contributors. This personal fork is maintained by [Accompany-cc](https://github.com/Accompany-cc).

Released under the [MIT License](./LICENSE). Upstream copyright and license notices are preserved in [Third-Party Licenses](./THIRD-PARTY-LICENSES.md).
