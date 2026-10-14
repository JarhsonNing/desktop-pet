# 自定义宠物皮肤

宠物造型、表情、动画、透明命中和气泡使用同一份 UI 定义。控制面板继续使用现有布局。默认麻薯也是数据皮肤，定义位于 `skins/default`，程序内嵌同一份文件作为回退；不再在 Rust 中定义身体、耳朵、眼睛等造型。

## 直接体验

控制面板 →「皮肤」：粘贴皮肤目录、`.mochi` 或 `.zip` 路径，点击「加载并使用」。也可以把皮肤包拖到该页的专用加载区域。拖到宠物上的文件和普通 ZIP 会收纳到中转站。选择会保存在配置中，下次启动继续使用。

编辑原来的目录文件后点击「再次加载」即可更新，不需要重新编译。加载失败时保留当前皮肤；启动时保存的皮肤丢失或损坏，则使用内嵌默认皮肤并显示原因。「恢复默认皮肤」清除自定义路径。

仓库提供三套模板，安装包中的 `Skins` 目录提供对应皮肤包：

| 模板 | 内容 |
| --- | --- |
| `skins/default` | 原始麻薯，矢量图层、十种状态、眼神和弹性动作 |
| `skins/blue` | 改调色板的示例，展示换色不需要改程序 |
| `skins/png` | PNG 图片皮肤，替换图片即可换角色，可扩展成逐帧动画 |

```powershell
cargo run -- --skin skins/blue --settings
cargo run -- --check-skin skins/default
cargo run -- --pack-skin skins/default --output dist/my-skin.mochi
cargo run -- --skin dist/my-skin.mochi --settings
cargo run -- --unpack-skin dist/my-skin.mochi --output dist/my-skin-source
```

已安装的 Windows 版本可将 `cargo run --` 替换为 `Mochi.exe`；macOS 使用 `Mochi.app/Contents/MacOS/desktop-pet`。打包、解包和校验都不创建 GUI 或桌面恢复租约。输出包文件和解包目标目录必须尚不存在，避免覆盖已有内容。

## 包格式 v1

自定义扩展名为 `.mochi`，它带有独立格式头：

| 字节 | 含义 |
| --- | --- |
| 0–7 | `MOCHISK` 加一个 NUL 字节 |
| 8–11 | 小端 `u32`，包格式版本，当前为 `1` |
| 12 起 | 完整 ZIP 负载，文件偏移相对于此负载 |

ZIP 内部布局如下；`manifest.json` 必须位于根目录：

```text
manifest.json
pet.json
images/
  idle-1.png
  idle-2.png
```

普通 ZIP 使用完全相同的内部布局，不需要自定义文件头。将打包命令的输出扩展名改为 `.zip` 即可创建 ZIP。加载根据文件头识别格式；`.mochi` 使用解包命令取出可编辑文件。

`manifest.json`：

```json
{
  "format": "mochi.skin",
  "version": 1,
  "id": "my-pet",
  "name": "My Pet",
  "author": "Your name",
  "ui": "pet.json"
}
```

`id` 使用 1–64 个英文字母、数字、连字符或下划线。`ui` 是包内相对路径。格式名和版本会经过校验，未知版本会拒绝加载。

## UI 文件与行为绑定

`pet.json` 的顶层字段：

| 字段 | 含义 |
| --- | --- |
| `canvas` | 宠物窗口的逻辑宽、高，范围 64–1024；整体大小再乘用户设置的缩放 |
| `interaction_bounds` | `[x,y,width,height]`，角色所在区域，用于多屏离屏恢复 |
| `gaze_origin` / `gaze_distance` | 眼神参考点及光标距离归一化尺度 |
| `chase_anchor` | 追逐时贴近光标的参考点 |
| `palette` | 命名颜色，值为 `#RRGGBB` 或 `#RRGGBBAA` |
| `states` | 状态对应的整体动画位移；必须定义 `idle` |
| `layers` | 从后往前绘制的图层；每层有唯一 `id`、默认 `node` 和可选的状态变体 |
| `bubble` | 气泡布局、字体大小、颜色、边框和恢复按钮位置 |

状态名绑定现有行为引擎：`idle`、`happy`、`grumpy`、`sleep`、`dragged`、`play`、`chase`、`hiding`、`dizzy`、`walk`。缺少某个状态的整体动画时使用 `idle`；图层缺少某状态变体时使用它的默认 `node`。

状态变体是完整节点替换，不是字段合并。这样每种表情可以使用不同形状或图片。`visible: false` 可以隐藏某状态中的图层。

一个矢量图层示例：

```json
{
  "id": "body",
  "node": {
    "interactive": true,
    "fill": "cream",
    "stroke": { "color": "ink", "width": 2.8 },
    "shape": {
      "type": "ellipse",
      "center": [130, 164],
      "radius": [64, 52]
    }
  },
  "states": {
    "sleep": {
      "interactive": true,
      "fill": "cream",
      "stroke": { "color": "ink", "width": 2.8 },
      "shape": {
        "type": "ellipse",
        "center": [130, 164],
        "radius": [64, 43.68]
      }
    }
  }
}
```

节点通用字段：`visible` 默认 `true`；`follow_scene` 默认 `true`，为 `false` 时不跟随整体弹跳（例如地面阴影）；`interactive` 默认 `false`；`offset` 默认 `[0,0]`。填充和边框可以引用调色板，也可以直接写十六进制颜色。

形状字段：

| `type` | 专用字段 |
| --- | --- |
| `ellipse` | `center`、`radius` |
| `polygon` | `points`，3–64 个点，按顺序组成凸多边形 |
| `line` | `points`，2 个点 |
| `quadratic` / `cubic` | `points`，分别 3 / 4 个贝塞尔控制点 |
| `rect` | `origin`、`size`、`radius`（圆角） |
| `text` | `center`、`text`、`size`，颜色使用 `fill`；不能设置为可交互 |
| `image` | `origin`、`size`、`frames`、`fps` |

坐标和大小可使用固定数字，也可使用动画绑定：

```json
{
  "base": 0,
  "sine": { "amplitude": 5, "speed": 8, "phase": 0, "absolute": false }
}
```

值为 `base + amplitude * sin(time * speed + phase)`，时间单位为秒，`speed` 为弧度每秒。`absolute: true` 对正弦值取绝对值，可制作跌落弹跳。眼神绑定示例：`{"base":108,"gaze":{"axis":"x","amount":3}}`。`sine` 和 `gaze` 可以同时使用。

整体动画放在 `states.<状态>.offset`，局部动画放在节点 `offset` 或形状坐标中。渲染与命中使用同一套时间、状态和变换。动画仍遵守引擎的待机、活跃、安静和省电重绘频率。

## PNG 与逐帧动画

```json
{
  "id": "sprite",
  "node": {
    "interactive": true,
    "shape": {
      "type": "image",
      "origin": [56, 86],
      "size": [168, 150],
      "frames": ["images/idle-1.png", "images/idle-2.png"],
      "fps": 8
    }
  }
}
```

单张图片也写入 `frames` 数组。多帧按声明顺序循环，可在 `layer.states` 中为不同状态绑定不同帧组。图片按 `size` 缩放；如需保持原始宽高比，应按比例设置尺寸。透明度小于 16/255 的像素穿透点击，命中随当前动画帧更新。图片纹理在加载皮肤时创建，不在每帧读取或解码文件。

## 气泡

气泡字段参照默认 `pet.json`：`rect` 为正常状态的位置和尺寸，`hidden_height` 为图标隐藏时的高度；`text_inset`、`font_size`、`radius`、`fill`、`text_color`、`stroke` 控制外观。`restore_button` 是相对于气泡的 `[x,y,width,height]`。

恢复按钮始终绑定程序的桌面恢复功能，不能通过皮肤改成其他操作。按钮必须位于隐藏状态气泡内，且至少为 60×18；气泡必须位于画布内。台词和互动逻辑继续由行为引擎提供。

皮肤中的文本与设置面板共享内嵌 Mochi Sans 字体，保留 Noto Sans CJK SC 的完整 Unicode 字符覆盖（44,810 个字符），支持常用简繁体和部分扩展区汉字。字体覆盖之外的字符仍可能缺字；图片中的文字不受此限制。

## 校验与资源边界

皮肤是数据和 PNG 资源，不执行脚本，也不能改变捣蛋授权或运行系统命令。加载时会校验版本、颜色、状态、坐标、尺寸、资源引用和恢复按钮布局。

包大小上限 16 MiB，展开后上限 32 MiB，单文件上限 8 MiB，最多 256 个目录/文件条目。PNG 单边最多 2048 像素，全部引用图片合计最多 12,582,912 像素；每个图片节点最多 64 帧、0.1–30 FPS。画布最多 1024×1024，图层最多 128 个。

包内路径统一使用 `/`，区分大小写。拒绝绝对路径、`..`、反斜杠、重复路径、符号链接及 Windows 保留设备名。运行时直接在内存中读取包；只有明确调用解包命令时才写出资源，且只写入新建的目标目录。
