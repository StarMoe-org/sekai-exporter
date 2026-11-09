# Glossary / 术语表

Terms used throughout the documents and the code. The Chinese term is the one used in every Chinese document.

文档与代码中统一使用的术语。中文文档一律使用此处的中文译名。

| English | 中文 | Definition / 定义 |
|---|---|---|
| object | 对象 | One instance of a Unity class in a serialized file, identified by `(bundle, path ID)`. 序列化文件中某个 Unity 类的一个实例，以 `(资源包, path ID)` 标识。 |
| type tree | type tree | The field layout of a class as stored in a file. 文件中记录的类字段布局。 |
| layout table | 布局表 | A field layout for classes stored without type trees, supplied separately. 为不带 type tree 的类单独提供的字段布局。 |
| PPtr | PPtr | A reference from one object to another: `(file index, path ID)`. 对象之间的引用：`(文件序号, path ID)`。 |
| object graph | 对象图 | All objects of a version and the references between them. 一个版本的全部对象及其相互引用。 |
| object document | 对象文档 | The JSON document of every object in a bundle, with references resolved. 一个资源包内全部对象的 JSON 文档，引用已解析。 |
| closure | 闭包 | A root object and every object reachable from it that an asset needs. 一个根对象，以及从它出发可以到达、且资产所需的全部对象。 |
| asset | 资产 | A self-contained result that a program outside the engine can use. 可由引擎以外的程序直接使用的自足成品。 |
| asset form | 资产形式 | A decision on how a closure is interpreted, in which container and for which runtime. 关于以何种方式解释闭包、写成何种容器、供何种运行时使用的决定。 |
| asset document | 资产文档 | The published record of one asset: form, converter, source objects, member files and their roles. 一个资产的发布记录：形式、转换器、来源对象、成员文件及其角色。 |
| converter | 转换器 | A pure function that turns a closure and table rows into an asset of one form. 将闭包与数据表记录转换为某种形式的资产的纯函数。 |
| encoder | 编码器 | A pure function that writes decoded content in one format with fixed parameters. 以固定参数将已解码内容写成某种格式的纯函数。 |
| input | 输入 | External bytes identified by their hash. 以哈希标识的外部字节。 |
| derivation | 派生 | One application of a pure function to inputs: `index`, `convert` or `transform`. 纯函数对输入的一次应用：`index`、`convert` 或 `transform`。 |
| fingerprint | 指纹 | The hash of a function's name and version, its parameters and the hashes of all its inputs. 函数名称与版本、参数以及全部输入哈希的哈希值。 |
| binding | 绑定 | A name `(region, version, view, path)` given to one output of one derivation. 赋予某个派生的某个输出的名称 `(区服, 版本, 视图, 路径)`。 |
| view | 视图 | A function that produces bindings for one consumer. 为特定使用者产生绑定的函数。 |
| materialization | 物化 | Writing a set of bindings in some shape to some place. 将一组绑定以某种形状写入某处。 |
| layout (publishing) | 布局（发布） | The shape of a materialization: `index` or `tree`. 物化的形状：`index` 或 `tree`。 |
| store | store | The content-addressed place where bytes are kept. 按内容寻址存放字节之处。 |
| catalog | 目录 | The index of inputs, derivations, bindings and publications. 输入、派生、绑定与发布的索引。 |
| listing | 列表 | All assets of one form in a region and version. 某区服某版本中某种形式的全部资产。 |
| evaluator | 求值器 | The part that decides when and where derivations are computed: eager or lazy. 决定派生何时、何处计算的部分：急切或惰性。 |
| logical path | 逻辑路径 | A container path without its fixed prefix and extension. 去掉固定前缀与扩展名的容器路径。 |
| region | 区服 | One released version of the game: `jp`, `en`, `tw`, `kr` or `cn`. 游戏的一个发行版本：`jp`、`en`、`tw`、`kr` 或 `cn`。 |
| source | 来源 | A supplier of inputs: `cp`, `nuverse`, `dir`, `apk` or `store`. 输入的提供者：`cp`、`nuverse`、`dir`、`apk` 或 `store`。 |
| secret | 密钥 | A key or account value read from the environment or a file outside the repository. 从环境变量或仓库外文件读取的密钥或账号值。 |
| runtime | 运行时 | The program that consumes an asset and defines its form. 消费资产并定义其形式的程序。 |
