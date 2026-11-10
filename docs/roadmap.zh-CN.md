# 路线图

[English](roadmap.md)

本页列出导出器的每个部分及其状态，方便所有人了解已有什么、认领未完成的工作。有规格的部分，每行都链接到定义它的规格。

## 状态

| 状态 | 含义 |
|---|---|
| 已完成 | 已有规格和实现，并有基于合成输入的测试覆盖 |
| 部分完成 | 已实现所列情形；其余情形报告为错误，绝不猜测 |
| 计划中 | 需要，尚未实现 |
| 待定 | 需要，但形式或读取方尚未确定；第一步是写规格 |

## 认领一项工作

1. 选一行状态为“计划中”或“待定”的项目，在 issue 中说明你正在做，避免两人重复劳动。
2. 如果该行还没有规格，先按[规格](spec/README.zh-CN.md)的要求写规格。规格要写明该形式面向的运行时，以及如何检验其正确性。
3. 代码放在哪里、遵守哪些规则、运行哪些检查，见[贡献指南](../CONTRIBUTING.zh-CN.md)。
4. 改变某行状态的 pull request 同时更新本页及其英文版。

## 读取

| 部分 | 状态 | 说明 |
|---|---|---|
| Unity 资源包：解析出对象图，所有跨文件引用均已解析 | 已完成 | [对象文档](spec/object-documents.zh-CN.md) |
| 客户端安装包（APK 与 XAPK），配合版本类表 | 已完成 | [来源](sources.zh-CN.md)；类表由使用者提供 |
| 客户端安装包内 `MonoBehaviour` 的字段 | 已完成 | 通过使用者提供的脚本布局表读取；没有布局的对象报告为 `no_layout` |
| 对象文档在原生与 WebAssembly 构建之间的逐字节比对 | 已完成 | |

## 解码与编码

| 部分 | 状态 | 说明 |
|---|---|---|
| 贴图：Alpha8、R8、RGB24、RGBA32、RGB565、ETC1、ETC2 RGB、ETC2 RGBA8、EAC R、crunch DXT5、ASTC LDR 与 HDR | 已完成 | 其他格式报告为不支持 |
| HCA 与 ADX 音频，ACB 与 AWB 归档，USM 容器 | 已完成 | |
| PNG、无损与有损 WebP、OpenEXR、FLAC、MP3、AAC | 已完成 | [功能开关](features.zh-CN.md#后端) |

## 流水线与发布

| 部分 | 状态 | 说明 |
|---|---|---|
| `export` 与 `update`：逐个资源包流式求值，写入内容寻址 store | 已完成 | [流水线](pipeline.zh-CN.md) |
| 选择性运行（`--only`） | 已完成 | |
| `index` 布局与发布文档 | 已完成 | [发布文档](spec/publication.zh-CN.md)、[资产文档](spec/asset-documents.zh-CN.md) |
| `tree` 布局 | 已完成 | [发布](publishing.zh-CN.md) |
| `storage` 视图 | 已完成 | [storage 视图](spec/storage-view.zh-CN.md) |
| S3 兼容存储 | 已完成 | [发布](publishing.zh-CN.md#s3-兼容存储桶) |
| WebAssembly 核心，持续集成中与原生构建比对 | 已完成 | |
| 惰性求值器（作为库） | 已完成 | [架构](architecture.zh-CN.md) |
| `serve`：以 HTTP 服务器提供惰性求值 | 计划中 | feature 已定义；有部署需要时再实现 |

## 通用形式

| 形式 | 状态 | 说明 |
|---|---|---|
| `texture` | 已完成 | [贴图](spec/texture.zh-CN.md) |
| `sprite` | 部分完成 | [精灵](spec/sprite.zh-CN.md)；图集完整持有的精灵会导出，裁边或旋转的图集精灵报告为不支持 |
| `audio`：CRI cue 表 | 已完成 | [音频](spec/audio.zh-CN.md) |
| `AudioClip` | 计划中 | 含有它的资源包报告为不完整 |
| `video` | 已完成 | [视频](spec/video.zh-CN.md) |
| `text` 与 `font` | 已完成 | [文本](spec/text.zh-CN.md)、[字体](spec/font.zh-CN.md) |
| `live2d` 模型与动作集 | 已完成 | [Live2D](spec/live2d.zh-CN.md)、[Live2D 动作](live2d-motion.zh-CN.md) |
| 模型文件（GLB），作为其他形式的成员 | 已完成 | [模型文件](spec/model.zh-CN.md) |
| 组件文档 | 已完成 | [组件文档](spec/component-documents.zh-CN.md) |
| Clip 文档 | 已完成 | [Clip 文档](spec/clip-documents.zh-CN.md) |
| 由 clip 生成 glTF 动画 | 计划中 | 目前由 clip 文档承载曲线 |
| `ugui`：UI 布局文档 | 待定 | feature 已定义，尚不转换任何内容；形式需要先有明确的读取方 |
| `timeline` | 待定 | feature 已定义，尚不转换任何内容 |
| `particles` | 待定 | feature 已定义，尚不转换任何内容；MySekai 特效已能解码其中的粒子系统 |

## 剧情

| 形式 | 状态 | 说明 |
|---|---|---|
| `scenario` | 已完成 | [剧本](spec/scenario.zh-CN.md) |
| `scenario-background` | 已完成 | [剧本背景](spec/scenario-background.zh-CN.md) |
| `action-set` | 已完成 | [动作集](spec/action-set.zh-CN.md) |
| `episode` | 已完成 | [话](spec/episode.zh-CN.md) |
| `virtual-live-script` | 已完成 | [虚拟演唱会剧本](spec/virtual-live-script.zh-CN.md) |

## 服装

| 形式 | 状态 | 说明 |
|---|---|---|
| `costume-model` | 已完成 | [服装](spec/costume.zh-CN.md#costume-model) |
| `costume-motion` | 已完成 | [服装](spec/costume.zh-CN.md#costume-motion) |
| `costume-role`：某个组合的某个角色能穿的全部部件 | 已完成 | [服装](spec/costume.zh-CN.md#costume-role) |
| `costume`：每套服装的各个造型 | 已完成 | [服装](spec/costume.zh-CN.md#costume) |

## MySekai

| 形式 | 状态 | 说明 |
|---|---|---|
| `mysekai-fixtures` | 已完成 | [家具](spec/mysekai-fixtures.zh-CN.md) |
| `characters` | 已完成 | [角色](spec/characters.zh-CN.md) |
| `client-config` | 已完成 | [客户端配置](spec/client-config.zh-CN.md) |
| `communication-actions` | 已完成 | [交流动作](spec/communication-actions.zh-CN.md) |
| `alone-actions` | 已完成 | [独处动作](spec/alone-actions.zh-CN.md) |
| `talks` | 已完成 | [对话](spec/talks.zh-CN.md) |
| `fixture-talks` | 已完成 | [家具对话](spec/fixture-talks.zh-CN.md) |
| `fixture-reactions` | 已完成 | [家具反应](spec/fixture-reactions.zh-CN.md) |
| `fixture-meta` 与 `fixture-areas` | 已完成 | [家具区域](spec/fixture-areas.zh-CN.md) |
| `emoticon` 与 `emoticons` | 已完成 | [表情](spec/emoticons.zh-CN.md) |
| `facial-table` 与 `facial-tables` | 已完成 | [表情表](spec/facial-tables.zh-CN.md) |
| `field-camera` | 已完成 | [场地相机](spec/field-camera.zh-CN.md) |
| `fixture-model`：家具及其材质、碰撞体与导航数据 | 已完成 | [家具模型](spec/fixture-model.zh-CN.md) |
| 家具挂点 | 计划中 | 角色和物品挂到家具上的位置 |
| 家具机关 | 计划中 | 家具中会动、可交互的部分 |
| 角色模型 | 计划中 | 基于与服装、家具共用的模型文件写出层 |
| 动作库 | 计划中 | 运行时播放的角色动作及其索引 |
| 场地模型 | 计划中 | 家园场地及其时间常量 |
| 天气与特效 | 计划中 | 基于 `emoticons` 已有的粒子解码 |
| 时间线与角色动画 | 计划中 | 依赖角色模型和家具 |
| UI 布局 | 待定 | 依赖客户端安装包内 `MonoBehaviour` 的字段；形式尚未确定 |
| 声音 | 已完成 | 使用通用的 `audio` 形式，没有单独的形式 |
