# 虚拟演唱会剧本

[English](virtual-live-script.md)

`virtual-live-script` 形式是虚拟演唱会其中一段所发生的内容——谁出现、移动与说话，灯光、声音与观众——写成 JSON 文档。转换器：`sekai-moesekai` 中的 `virtual-live-script`（feature `virtual-live`）。形式版本：1。

## 1. 运行时

显示或搜索虚拟演唱会中所说与所做的内容的阅读器，以及重放它的播放器。形式按游戏的存储给出剧本；它不把所含的名称解析为文件，也不把它与列出虚拟演唱会的游戏数据表连接。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `script` | JSON 文档，扩展名 `json` |

文档是一个对象，按存储顺序保存剧本对象的每个序列化字段，但排除每个 `MonoBehaviour` 都有的四个字段：`m_GameObject`、`m_Enabled`、`m_Script` 与 `m_Name`。值的写法与 [`scenario`](scenario.zh-CN.md#2-内容) 形式相同。剧本的事件按种类分为数组，例如 `characterTalkEvents` 与 `lightEvents`；每个事件给出其时刻，以秒计。

## 3. 来源

**根对象：** 脚本为 `Assembly-CSharp/Sekai.Core.VirtualLive.MasterOfCeremonyData` 的每个 `MonoBehaviour`（类 114）。形式不读取游戏数据表。

**读取的字段：** 根对象的全部序列化字段。

**文档以外的结果：** 即 [`scenario`](scenario.zh-CN.md#3-来源) 形式所述的那些。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。

## 5. 正确性

文档不加解释地保存对象的所存值，且同一对象总是给出相同的字节。一项测试在内存中构建一个剧本对象并比较字节；在真实资源包上，该脚本的每个对象都恰好读到其末尾。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
