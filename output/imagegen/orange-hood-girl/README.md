# 橘猫斗篷女孩 PNG 皮肤

依据用户提供的人物三视图，使用 `imagegen` 技能的 API/CLI 模式、`gpt-image-2` 生成十种状态，每种六帧。完整提示词在 `prompts/`，参考图为 `reference.png`，生成参数与调用结果在 `generation-plan.json` 和 `generation-results.json`。生成使用用户指定的代理；项目文件不包含请求凭据。

角色保持二维 Q 版插画风格、金橙色头发、猫帽斗篷、白色上衣、深色短裤和猫咪短袜。动作按双足人形角色制作，头发、斗篷与服装尾巴参与动作。

## 文件与使用

- 项目源码：`skins/orange-hood-girl/manifest.json`、`pet.json`、`images/*.png`。
- 加载包：`dist/skins/orange-hood-girl.mochi`。
- 素材 ZIP：`dist/orange-hood-girl-assets.zip`，含可编辑皮肤、透明精灵图、参考图、提示词、GIF 和测试报告。
- `sheets/` 保存模型原始输出，`transparent-sheets/` 保存清理透明噪声后的图；都是三列两行，帧顺序为先上行后下行，每行从左到右。
- `frames/<state>/000.png` 至 `005.png`：60 张 256×256 RGBA PNG。
- `native-<state>/animation.gif` 与 `report.json`：实际 `SkinRenderer` 捕获的动画及检查报告。
- `native-happy/gallery.png`：实际渲染器的十状态画廊。
- `app-load.png`：使用新建隔离配置的正常应用加载截图。

控制面板「皮肤」页选择 `.mochi`，或从项目根目录运行：

```powershell
cargo run -- --skin skins/orange-hood-girl --settings
```

| 状态 | 动作 | 素材播放 FPS |
| --- | --- | --- |
| idle | 待机、表情与尾巴轻动 | 2 |
| happy | 笑眼招手 | 8 |
| grumpy | 抱臂、转头闹脾气 | 4 |
| sleep | 抱膝坐着睡觉 | 2 |
| dragged | 悬空、腿和斗篷摆动 | 6 |
| play | 蹲下拨弄毛线球 | 6 |
| chase | 原地小跑 | 12 |
| hiding | 蹲下拉低帽沿、偷看 | 4 |
| dizzy | 同一只手扶额头、另一只手下垂、轻轻晃头 | 6 |
| walk | 双足原地迈步 | 8 |

## 处理和验证

模型返回的图片已有透明通道，因此保留原始 alpha，只清除 alpha ≤8 的不可见噪声，没有对角色执行背景去色。按固定网格切帧并统一缩小；睡眠、玩耍、躲藏素材使用固定的状态缩放，修正模型在低姿态中放大的角色比例，不对各帧独立紧裁。

所有状态都有独立素材，没有复用待机替代缺失状态。共 3,932,160 图片像素，满足当前格式的 4M 总像素预算。源码与 `.mochi` 均通过原生校验，动画边界采样无警告。

十种状态各录制三秒、12 FPS 的原生预览，每段 36 个捕获帧、六张不同画面；循环接缝检查均未触发警告。正常应用加载退出码为 0，截图成功；使用隔离配置，未启用捣蛋。详见 `preview-results.json`。

这是六帧动画初版；状态切换直接换帧组，没有跨状态姿态插值。生成图有少量手绘动作线，帧间轮廓也有手绘差异；格式和接缝检查不代表逐帧美术完全一致。预览按 12 FPS 采样，应用待机仍按既有 2 FPS 预算重绘。

头晕状态已按反馈重画，原图与补帧各帧保持同一只手扶额头，另一只手下垂。修正提示词、逐帧对照与新验证报告见 `dizzy-repair/`。
