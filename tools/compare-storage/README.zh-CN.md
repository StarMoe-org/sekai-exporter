# 目录树比对

[English](README.md)

这些脚本比对两个目录树，并逐字节比对同一输入的两个输出目录。只使用 Python 标准库。Pillow 与 ffprobe 为可选项。

## compare.py

```bash
python compare.py A B [--known known.tsv] [--ffprobe PATH] [--report out.json]
```

`A` 是被测树，`B` 是参照树。每个相对路径（统一使用正斜杠，按排序处理）被归为以下几类：

- `missing`：路径只存在于 B 中。
- `extra`：路径只存在于 A 中。
- `same`：内容被判定为相同。
- `different`：内容被判定为不同。

判定依据文件类型：

- `.json`：解析两个文件并比较其值。忽略对象的键顺序，数值按解析后的值比较，因此 `1` 与 `1.0` 相等。布尔值永远不等于数字。报告第一处不同的路径，例如 `$.a[3].b`。
- `.png`、`.webp`、`.jpg`：字节相同则为 `same`。否则，若安装了 Pillow，则两张图都解码为 RGBA 后逐像素精确比较，报告尺寸不一致或不同像素的数量。未安装 Pillow 时，只比较文件头中的格式、宽度与高度；字节不同的图片判为 `different`，并注明未比较像素。
- `.mp3`、`.m4a`、`.aac`、`.mp4`、`.ogg`：字节相同则为 `same`。否则对两个文件运行 `ffprobe -v error -show_streams -show_format -of json`，比较格式契约：容器格式名；每条流的编码、采样率、声道数、宽与高（按流的顺序）；时长；以及按 32 kb/s 分档（四舍五入到最近的档）的码率。时长允许相差参照树流的一帧：视频为 `1 / 帧率`，AAC 音频为 `1024 / 采样率`，MP3 音频为 `1152 / 采样率`，其他情况为 0.05 秒。报告第一个不同的字段。
- 其他文件：比较 SHA-256。

`--ffprobe` 指定 ffprobe 可执行文件。文件名以 `.py` 结尾的命令会用当前的 Python 解释器运行。默认在 `PATH` 中查找 `ffprobe`。没有 ffprobe 时，不同的媒体文件报告为 `different`。

输出先列出各类别的数量，再按类别和顶层目录分组列出不一致的路径。位于顶层的文件归入 `(top level)`。`--report` 将同样的分组写为 JSON。

退出状态：当每个 missing、extra 或 different 路径都被已知差异解释时为 0；有任何路径未被解释时为 1；用法错误为 2。

## 已知差异

`--known` 读取人工维护的 TSV 文件，每行一个 `pattern<TAB>reason`：

```text
# 以井号开头的行为注释
logs/*.txt	运行时写入时间戳
legacy/old-name.bin	为兼容性检查而保留
```

pattern 是 `fnmatch` 通配符，按区分大小写的方式匹配相对路径。匹配某个 pattern 的 missing、extra 或 different 路径会按该原因报告，不会导致失败。相同的路径从不作为已知差异报告。空行会被跳过。

## determinism.py

```bash
python determinism.py RUN1 RUN2 [--metrics FILE]
```

`RUN1` 中的每个文件都必须在 `RUN2` 中存在且字节相同，反之亦然。脚本列出缺失和不同的路径。使用 `--metrics` 时，`FILE` 是 JSON Lines 文件，每行一个对象，通常是第二次运行的 `metrics.jsonl`。`convert` 阶段的每个 span（`{"kind": "span", "stage": "convert"}`）计为一次重新计算的派生，其数目必须为 0。退出状态：全部一致为 0，否则为 1；用法错误为 2。

## 测试

在仓库根目录运行：

```bash
python -B -m unittest discover -s tools/compare-storage -v
```

测试在临时目录中由生成的字节构建所有目录树。媒体测试使用假的 ffprobe 脚本，因此不需要真正的 ffprobe。未安装 Pillow 时，需要 Pillow 的测试会被跳过。
