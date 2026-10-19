# Slack / Discord Markdown：第三方库选型

核验日期：2026-10-19。本文核对包注册表、维护者仓库、实际源码和测试文件；没有安装候选依赖、运行第三方代码或修改运行配置。版本与活动日期是当日快照，不能视为持续维护保证。

## 建议

**有可用的第三方库，但没有核验到一个可直接替换 PocketPilot 两端适配、同时满足当前边界的成熟 Rust 成品。**建议继续使用已经锁定的 `pulldown-cmark` 解析 Markdown，共享文档结构；Slack / Discord 只保留各自的渲染、长度和通知策略。这样复用的是成熟语法解析器，并非自行编写 Markdown 解析规则。

如果项目本身是 Node.js，`slackify-markdown` 是值得优先评估的现成 Slack 转换器；本项目没有必要仅为它引入 Node.js 服务或子进程。可参考其转换策略与测试用例，但复制源码或测试内容时须保留 MIT 许可要求。[N1][N2]

## 解析器、平台转换器与 SDK 不是同一层

| 层次 | 负责什么 | 候选 | 对本项目的结论 |
| --- | --- | --- | --- |
| 通用 Markdown 解析 | 将原文解析为段落、强调、链接、代码、列表等结构 | `pulldown-cmark`、`comrak` | 直接复用现有解析器 |
| Markdown → Slack 格式 | 将解析结构或原文转换为 Slack 所用方言 | `slackify-markdown`、`slack-markdown-converter` | 前者成熟度证据更充分，但需要 JS；后者需补足验证 |
| 流式修复与多目标渲染 | 修复未闭合语法、生成平台格式、切分 | `mdwire-core` | 有参考价值，当前 Slack 目标和本项目不同，且没有 Discord 目标 |
| Discord 方言解析/生成 | 解析 Discord 专有语法或构造该语法字符串 | `discord-md` | 不能等同于 CommonMark → Discord 完整转换器 |
| SDK 与消息 builder | 构造请求、blocks、组件、转义局部内容 | `slack-morphism`、`serenity` | 继续使用，但不能替代通用 Markdown 解析 |

表中能力和限制依据后文对应的包元数据、源码及 README。是否可靠取决于所需功能、边界测试与许可；下载量或版本新旧本身不足以证明可靠。

## Rust 通用解析：保留 pulldown-cmark

`pulldown-cmark` 当前稳定版为 **0.13.4**，2026-05-20 发布，MIT，声明 MSRV 1.71.1；PocketPilot 的 lockfile 已使用该版本。维护者将其定义为 CommonMark pull parser，提供可选 GFM 表格、任务清单、删除线等扩展。仓库有 CommonMark/GFM spec fixtures、回归测试、CJK emphasis 测试及 CI；截至核验日未归档且仍有提交。[R1][R2][R3]

这些证据支持继续用它承担语法解析。它的内置 HTML 输出并不意味着有内置 Slack / Discord 输出；在解析事件之上实现薄适配器，仍需处理平台语义，例如 Slack 粗体符号、链接语法和不支持结构的降级。

`comrak` 是可行替代：**0.56.0**，2026-10-06 发布，BSD-2-Clause，MSRV 1.89；提供 CommonMark/GFM AST、HTML/CommonMark 等输出，仓库有规范、回归、fuzz、MSRV 和 CI 测试。它适合需要更复杂 AST 改写的场景，但也没有因此直接获得 Slack / Discord 成品适配器。当前需求没有证明需要替换现有解析器，避免同时维护两套解析结果。[R4][R5][R6]

## Slack 成品：slackify-markdown 最值得参考

### slackify-markdown：可以用，但不适合直接引入当前 Rust 运行链

核验到 npm **5.0.0**，2025-11-07 发布，MIT，维护者仓库未归档。该版本要求 **Node.js 22+、ESM**；使用 `unified`、`remark-parse`、`remark-gfm` 和 `remark-stringify`，实际源码通过 AST 管线转换，而非对所有原文做简单正则替换。[N1][N2][N3]

仓库有 Vitest 测试和 CI，实际测试覆盖标题、粗斜体、删除线、列表、引用式链接、转义、图片、代码块、语言标记、提及和引用等。这比“README 声称支持 Markdown”提供了更强证据。但本文没有运行该测试集，没有量化覆盖率，也没有证明它满足本项目的通知、安全链接、长度和原文保真策略。[N2][N4]

结论：**JS 项目可以优先评估；本项目作为参考和合成用例来源。**它返回 Slack 格式字符串，不负责 workspace → 项目 → session 导航、持久 outbox、Block Kit 菜单、按钮权限或文件附件。即使采用，平台适配和投递逻辑仍然存在。

### slack-markdown-converter：确实存在，暂不推荐直接采用

crates.io 包 **0.1.0**，2026-03-25 发布，MIT。README 声称支持标题、强调、链接、表格降级及 fenced code。检查源码可见依赖 `regex`，自行扫描代码 fence、表格和行内标记，没有复用 CommonMark AST。[R7][R8]

核验到的仓库树只有 Cargo 文件、README、许可和 `src/lib.rs`；未发现 CI 或独立测试目录，源码也未发现 `#[test]` 测试。仓库活动集中在初次发布日。源码的链接正则仅匹配部分 `http(s)` 行内链接，不能据此认定支持 CommonMark 全部链接与嵌套语法。[R8]

结论：**可阅读表格降级思路，暂不接入。**引入它会把成熟解析器的一部分职责改交给未经本项目边界测试覆盖的规则，而非减少平台适配的总工作量。

### mdwire-core：活跃但较新，目标并不匹配

crates.io **0.1.14**，2026-10-10 发布，MIT。仓库创建于 2026-09-17，核验日仍活跃，核心无外部依赖；存在 rendering tests、corpus、fuzz、跨实现一致性测试和 CI。它重视流式语法修复、结构完整的切分及修复报告，值得关注。[R9][R10]

不过其 README 明确说明 Slack 目标是 **`markdown_text` 标准 Markdown**，不是当前 PocketPilot 的普通 `mrkdwn` blocks；列出的七个目标也不含 Discord。源码的 Slack 强调输出为 `**`，进一步印证两条路径不同。自动修复输入也意味着展示文本可能被改写，需要对保真和导出另行约定。[R10][R11]

结论：**观察和参考，不作为本轮两端通用转换依赖。**不能因为包描述提到 Slack 就把其结果放入现有 `mrkdwn` section。未来若确实采用 Slack 标准 Markdown 路径或做 token 级流式展示，再以同一套合成语料独立评估。

### 名称与方向核验

- **`@slack/markdown-to-blocks`：**当日 npm registry 查询返回 404，未核验到可安装包；不要将它作为 Slack 官方现成库推荐。这是本次查询结果，不是对所有私有 registry 或未来发布的断言。[N5]
- **`slack-blocks-render`：**真实 Rust 包，但方向是 Slack blocks → Markdown，与本项目所需方向相反。[R12]
- **`markdown-to-slack`：**npm 能查到同名包，但该版本包元数据没有 repository 字段；本次没有继续完成来源和测试核验，因此不因名称合适而优先推荐。[N6]

## Discord：方言生成器有用，仍需要 CommonMark 映射

`discord-md` **3.0.0**，2023-11-12 发布，MIT OR Apache-2.0；仓库截至核验日未归档，2026-06 仍有活动。提供 Discord Markdown AST、parser 和 generator，并有 parse/generate 往返测试与 CI。[R13][R14]

README 同时明确列出限制：parser 不解析 block quotes，嵌套和词内强调可能与客户端不同，escape sequences 作为普通文字处理。它展示的支持范围以强调、下划线、删除线、spoiler、代码和引用生成为主，不能推断已经覆盖当前 Discord 所有标题、列表等语法，更不能直接把它当作任意 CommonMark 文档的完整转换器。[R14]

其他搜索结果也需要区分用途：`discord-markdown` 的发布记录停留在 2021 年；`markdown_converter` 最后发布于 2024 年，描述含 flavored Markdown → Discord，但当前注册表许可字段为 `non-standard`，采用前需读取并确认具体许可；官方仓库 `discord/discord-intl` 下的 `discord_intl_markdown` 虽有新版本，但描述是 ICU messageformat 与 Markdown 的 i18n 解析器。以上元数据不能单独证明它们适用于本项目的通用正文转换。[R15][R16][R17]

结论：**继续用通用解析器获得语义，再生成 Discord 支持的子集。**它比 Slack 更接近常见 Markdown，但仍要明确表格降级、代码 fence、标题文本转义、链接协议、消息长度和通知策略。平台能力见 [两端格式调研](chat-formatting.md)。

## 已有 SDK：复用组件，不能误认为全文转换器

`slack-morphism` 的 Block Kit 类型和 `serenity` 的消息/组件 builder 负责构造平台 payload；给字段设置 `mrkdwn` 或调用 `content` 不会自动转换 CommonMark 文档。[K1][K2]

特别是 Serenity 的 `MessageBuilder::push_safe` 只做其定义的 normalization 及部分字符转义；`push_codeblock_safe` 会将输入中的三反引号替换为空格。这适合特定展示场景，却不能作为保持代码原文的通用转换函数。`safe` 名称不等于本项目所需的“完整格式兼容、禁止通知、内容保真”。正文路径仍须保留 `allowed_mentions`，用户文件附件与文字分开处理。[K2]

## 本项目的接入边界

1. **复用语法解析器。**用锁定的 `pulldown-cmark` 解析成公共文档结构；启用哪些扩展由项目明确决定。
2. **保留薄平台适配。**Slack 用结构化菜单和普通 `rich_text` 正文，Discord 输出其 Markdown 子集；平台相关的长度、转义、mentions 和附件独立处理。
3. **用候选库校验策略。**用相同合成输入比较链接、嵌套强调、中文标点、反引号、表格、任务清单及 emoji；不同输出需判断语义，不把第三方输出当唯一标准。
4. **满足边界再采用依赖。**若未来某库在既定语料、许可、维护和运行成本上明确优于薄适配，再替换渲染层，不连带改投递、权限或引入 Node 服务。候选元数据核验不等于编译、客户端显示或真实消息投递已经验收。

## 一手来源

- [R1：pulldown-cmark 包元数据](https://crates.io/api/v1/crates/pulldown-cmark)
- [R2：pulldown-cmark 仓库与 README](https://github.com/pulldown-cmark/pulldown-cmark)
- [R3：pulldown-cmark spec / tests](https://github.com/pulldown-cmark/pulldown-cmark/tree/main/pulldown-cmark/tests)
- [R4：comrak 包元数据](https://crates.io/api/v1/crates/comrak)
- [R5：comrak 仓库与 README](https://github.com/kivikakk/comrak)
- [R6：comrak tests](https://github.com/kivikakk/comrak/tree/main/src/tests)
- [R7：slack-markdown-converter 包元数据](https://crates.io/api/v1/crates/slack-markdown-converter)
- [R8：slack-markdown-converter 源码](https://github.com/guoqingbao/slack_markdown_converter/blob/main/src/lib.rs)
- [R9：mdwire-core 包元数据](https://crates.io/api/v1/crates/mdwire-core)
- [R10：mdwire 仓库与 README](https://github.com/minjun0219/mdwire)
- [R11：mdwire 平台输出策略源码](https://github.com/minjun0219/mdwire/blob/main/crates/mdwire-core/src/vocab.rs)
- [R12：slack-blocks-render 包元数据](https://crates.io/api/v1/crates/slack-blocks-render)
- [R13：discord-md 包元数据](https://crates.io/api/v1/crates/discord-md)
- [R14：discord-md 仓库与限制说明](https://github.com/ciffelia/discord-md)
- [R15：discord-markdown 包元数据](https://crates.io/api/v1/crates/discord-markdown)
- [R16：markdown_converter 包元数据](https://crates.io/api/v1/crates/markdown_converter)
- [R17：discord_intl_markdown 包元数据](https://crates.io/api/v1/crates/discord_intl_markdown)
- [N1：slackify-markdown npm 元数据](https://registry.npmjs.org/slackify-markdown)
- [N2：slackify-markdown 仓库与运行要求](https://github.com/jsarafajr/slackify-markdown)
- [N3：slackify-markdown AST 管线源码](https://github.com/jsarafajr/slackify-markdown/blob/master/src/index.ts)
- [N4：slackify-markdown 测试](https://github.com/jsarafajr/slackify-markdown/blob/master/__test__/slackify-markdown.test.ts)
- [N5：所查询的 @slack/markdown-to-blocks registry 地址](https://registry.npmjs.org/@slack%2Fmarkdown-to-blocks)
- [N6：markdown-to-slack npm 元数据](https://registry.npmjs.org/markdown-to-slack)
- [K1：slack-morphism 2.31.0 Block Kit 类型](https://docs.rs/slack-morphism/2.31.0/slack_morphism/models/blocks/index.html)
- [K2：serenity 0.12.5 MessageBuilder 源码](https://github.com/serenity-rs/serenity/blob/v0.12.5/src/utils/message_builder.rs)
