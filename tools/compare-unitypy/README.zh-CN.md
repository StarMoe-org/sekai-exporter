# compare-unitypy

将对象文档与 [UnityPy](https://github.com/K0lb3/UnityPy) 从同一批资源包中读出的字段相比对。两个读取器的输出均映射到[对象文档规范](../../docs/spec/object-documents.zh-CN.md)的值模型，要求每个对象的每个字段均相等。

```bash
pip install UnityPy==1.25.4
sekai-exporter objects <资源包目录> <文档目录>
python compare.py <资源包目录> <文档目录>
```

脚本按类输出对象数与存在差异的对象数，每个类至多列出两处差异。任一对象存在差异时，退出状态为 1。

持续集成在 `crates/sekai-unity/tests/fixtures` 中的样例资源包上运行该脚本；对其他任何资源包目录也可按同样方式运行。

UnityPy 对部分值的写法与规范不同，脚本在比对之前先作映射：

| UnityPy | 规范 |
|---|---|
| 8 位整数数组写作数字列表 | 字节载荷 |
| map 之外的 `pair` 写作元组 | 含 `first` 与 `second` 的结构体 |
| PPtr 写作 `{m_FileID, m_PathID}` | `null` 或引用 |
| 64 位整数写作数字 | 十进制字符串 |
