# 贡献指南

[English](CONTRIBUTING.md)

欢迎各类贡献，范围从修复一个缺陷到新增一种完整的资产形式。开始之前，请先阅读[对象与资产](docs/objects-and-assets.zh-CN.md)。该文档说明了项目的构造方式，以及各类改动应当位于何处。

## 改动的位置

| 目标 | 新增或修改的内容 |
|---|---|
| 新的资产形式 | 一个转换器模块，由其自身的 feature 启用，并在 `docs/spec/` 中编写对应的规范 |
| 已有资产的另一种组织方式 | 一个视图模块，由其自身的 feature 启用 |
| 支持读取器当前拒绝的某种格式变体 | 解包或解码 crate，并附带一个能够触发该变体的合成文件 |
| 修复缺陷 | 出现缺陷的模块，并附带一个在修复前失败的测试 |

## 规则

- **仓库中不得包含游戏数据。** 包括资源包文件、导出的图片、音频、模型、数据表与反编译代码；测试、样例与 issue 附件同样适用。测试输入须以合成方式构造，或使用明确允许再分发的文件。
- **规范先于实现。** 新的资产形式须附带一份文档，说明导出的内容、所依赖的对象与数据表、写出的格式，以及结果为何正确。
- **错误须显式报告。** 转换器无法产出正确的资产时，须对该资产报告具名错误，不得写入推测的值。
- **函数体内不得出现 `#[cfg]`。** feature 仅用于启用整个模块或可选依赖。
- **输出须确定。** 相同的输入产生相同的字节，与线程数无关。
- **不使用 `unsafe` 代码。** 工作区以 `forbid` 级别禁止 `unsafe_code`。

## 检查

提交 PR 之前，请运行：

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
cargo hack check --workspace --each-feature
```

最后一项将每个 feature 单独编译一次，用于发现 feature 之间未声明的依赖。

## 提交

提交信息使用英文的 [Conventional Commits](https://www.conventionalcommits.org/) 格式，类型小写，并写明改动内容及其可见效果，例如 `feat(export): write sprite atlases as separate images`。

## 许可

提交贡献即表示同意以本仓库的 MIT 许可证授权所贡献的内容。
