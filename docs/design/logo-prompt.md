# PocketPilot logo 生成 Prompt

用于生成项目 logo 和 Telegram bot 头像。下面的英文 prompt 可直接复制到图片生成工具；本仓库尚未生成或安装 logo。

```text
Design an original, production-ready symbol-only logo for PocketPilot.

Product context:
PocketPilot is a small developer utility for managing and continuing AI coding sessions from chat, across a local computer and remote SSH hosts. Its personality is calm, capable, portable, and reliable. The name combines a pocket-sized tool with the idea of a pilot helping you navigate ongoing work.

Core visual idea:
Create one unified mark that combines a simple pocket silhouette with a compact navigation pointer formed through negative space. Explore how the two shapes can share a contour so the result feels like a single recognizable symbol. Suggest direction and control without resembling a paper airplane. The mark should be distinctive even without the product name.

Art direction:
Clean, precise, vector-friendly geometry with balanced optical weight and a small amount of warmth. Use a restrained two-color palette with strong contrast. Favor a clear silhouette and generous negative space. The design must remain recognizable at 24–32 pixels and work in one color.

Composition and output:
Deliver a single centered logo on a flat, solid background, in a 1024 × 1024 square image. Keep approximately 18% clear space around the mark so a circular Telegram avatar crop does not clip it. Show only the finished symbol, with no presentation board, app mockup, variations, labels, or explanatory text.

Avoid:
No wordmark or lettering. No gradients, 3D rendering, shadows, glow, textures, tiny details, or decorative border. Avoid generic AI sparkles, brains, robots, circuit boards, terminal windows, and clusters of multiple icons. Do not reproduce or closely resemble the OpenAI, Claude, Telegram, Discord, or Rust logos. Do not use a paper airplane as the main shape. No watermark.

Before finalizing, check that the negative space stays open at small sizes, the symbol survives a circular crop, and its silhouette still works in monochrome.
```

若后续需要字标，另行生成：保持选定图形不变，搭配准确拼写的 `PocketPilot`；不要把字标压进 Telegram 头像。SVG 应在图形方向确认后重绘，不能把栅格图片改后缀当成矢量文件。
