# 项目内聊天 UI

这里保存可直接修改的 TypeScript 组件和 CSS，复用 Pi web UI **0.75.3** 的源码，不再安装或导入 `@earendil-works/pi-web-ui`，也不需要 `patch-package`。模型执行仍由包内 Pi 1.2.0 经 RPC 完成。

## 修改入口

| 文件 | 内容 |
| --- | --- |
| `components/AgentInterface.ts` | 居中空白态、消息区域、输入区域及滚动 |
| `components/MessageEditor.ts` | 输入框、发送/停止、图片附件、中文组合输入 |
| `components/Messages.ts`、`MessageList.ts` | 消息气泡、默认折叠的工具状态/详情与消息列表 |
| `dialogs/` | 模型选择和图片预览 |
| `tools/renderers/` | 工具进度与结果展示 |
| `styles/themes/default.css` | 颜色、字体、圆角及阴影变量 |
| `styles/chat.css` | Tailwind 入口、聊天公共样式 |
| `styles/utils/` | Markdown、代码高亮及公式样式 |

输入区附近的会话、配置和陪伴操作，以及宿主连接在上一级 `main.ts`，融合样式在 `page.css`。模型、MCP 表单/JSON 和 Skills 管理统一在 `panel/src/AgentPreferences.vue`；旧的独立设置弹窗和浏览器存储接口已移除。组件继续使用 Lit 和 mini-lit 基础控件；Markdown/代码块使用 mini-lit，图标使用 Lucide，公式使用 KaTeX。Vite 从本目录和 mini-lit 基础控件生成 Tailwind 样式，新增 utility class 会随构建生效；不要编辑 `panel/dist`。

本地代码已包含原先依赖补丁：RPC 鉴权、Pi 模型目录、仅图片附件和 IME Enter 防误发。没有引入浏览器模型执行、IndexedDB 后端、自动模型发现、文档解析、REPL 或 Artifacts。配置通过宿主接口保存，会话历史仍以 Pi JSONL 为准；sessionStorage 仅暂存未发送的输入文本，便于切换设置后继续编辑。

## 来源及许可

- 组件、工具展示和配置存储接口：[Pi v0.75.3 / packages/web-ui/src](https://github.com/earendil-works/pi/tree/v0.75.3/packages/web-ui/src)，MIT，Copyright (c) 2025 Mario Zechner。
- 主题、Markdown/高亮/公式 CSS：`@mariozechner/mini-lit@0.2.0` 的 `src/styles`，来源 [mini-lit](https://github.com/badlogic/mini-lit)，MIT，Copyright (c) 2025 Mario Zechner。
- 两者完整许可文本均保存在本目录的 `LICENSE`；分发脚本会将该文件及本说明收录到 `ThirdParty/mochi-chat-ui/`。

本目录只收录当前聊天页面需要的源码，并按麻薯 RPC 接口调整；后续直接维护本地代码，不从 npm 包复制或在安装时覆盖文件。
