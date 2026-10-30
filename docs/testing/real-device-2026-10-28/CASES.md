# 真机测试用例目录

基线 `ef5fd8cd6cb1c529a001d60bcef8d3e0923a21d9`；391 个稳定Case ID，覆盖当前能力清单 119 项。所有结果初始 NOT_RUN。

命令模板的 `<...>` 必须替换为现场观察值；`RECIPE:` 指向 SETUP.md 的步骤。PowerShell 中 `$U` 每次只代表一台明确手机。
分组的前置/清理是每一条用例的组成部分。组合用例中的每个子步骤均需结果：任一未执行不能将整条判 PASS，发现独立问题使用 `CASE_ID.a/b` 子检查记录并建issue。
权限和适用性先按 README.md / CLASSIFICATION.md 判断；模板包含的危险命令是登记范围，不是执行授权。

| 分组 | 用例数 | 系统 |
| --- | ---: | --- |
| [ENV](#env) 版本、构建和测试约束 | 7 | HOST |
| [DISC](#disc) 发现、身份和主机/目标诊断 | 10 | BOTH |
| [INFO](#info) 设备信息、Lockdown值与心跳 | 33 | BOTH |
| [PAIR](#pair) 已有信任与显式配对 | 5 | BOTH |
| [DDI](#ddi) 开发镜像、个性化与Cryptex | 15 | BOTH |
| [TUN](#tun) 现代隧道、RSD、管理与权限 | 13 | MODERN |
| [APP](#app) 应用、安装、进程控制 | 28 | BOTH |
| [AFC](#afc) AFC、HouseArrest与CoreDevice文件 | 21 | BOTH |
| [BACK](#back) MobileBackup2与本地备份工具 | 14 | BOTH |
| [LOG](#log) 日志、抓包、崩溃和符号 | 20 | BOTH |
| [DIAG](#diag) 诊断、性能与调试 | 43 | BOTH |
| [UI](#ui) 截图与SpringBoard | 14 | BOTH |
| [CONTROL](#control) 现代设置、旋转和Display | 22 | MODERN |
| [CLIP](#clip) 设备剪贴板与模拟定位 | 12 | MODERN |
| [VIEW](#view) Web Viewer、VNC与双向图像 | 14 | MODERN |
| [WEB](#web) WebInspector、CDP固定客户端与WebDriver | 23 | BOTH |
| [XCT](#xct) XCTest、WDA与统一工作流 | 21 | BOTH |
| [NET](#net) 端口、通知、伴侣与网络profile | 15 | BOTH |
| [PROFILE](#profile) 配置、Provisioning与受监督功能 | 21 | BOTH |
| [ACT](#act) 激活、恢复与明确搁置范围 | 21 | BOTH |
| [SDK](#sdk) Rust、Python及C公共接口 | 13 | BOTH |
| [CROSS](#cross) 两机、状态切换和完成复核 | 6 | DUAL |

<a id="env"></a>
## ENV 版本、构建和测试约束

前置：尚未执行任何设备写入。
清理：保留版本清单；不上传凭据/设备原始数据。
对照索引：`R-NONE`，详见 UPSTREAM.md；覆盖：。

### ENV-001 锁定本次源码与上游版本

- 适用：`HOST`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SETUP.md 的基线采集`
- 检查：记录HEAD、工作区dirty、origin/master缓存SHA、二进制SHA256、Rust/Go/Python/Node/FFmpeg与两个上游HEAD及dirty；不能只记录版本号
- 对照：`R-NONE`。

### ENV-002 本次源码构建CLI与绑定

- 适用：`HOST`；优先级 `P0`；权限 `READ`；外层时间上限参考 `1800s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SETUP.md 的构建与依赖`
- 检查：构建成功；实际执行路径来自本仓库；Python module/DLL和go-ios均有独立来源证据
- 对照：`R-NONE`。

### ENV-003 CLI全帮助与语法核对

- 适用：`HOST`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --help`
- 检查：逐个顶层和子命令--help留存；参数与cases一致；--help不是设备功能通过
- 对照：`R-NONE`。

### ENV-004 双机登记与作用范围

- 适用：`HOST`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SETUP.md 的设备及素材登记`
- 检查：D1实际iOS<17、D2实际iOS>=17；记录build与设备别名；允许写入的app/目录/设置和恢复方法明确
- 对照：`R-NONE`。

### ENV-005 测试素材完整性

- 适用：`HOST`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SETUP.md 的fixture清单`
- 检查：IPA v1/v2、签名runner/xctestrun、测试网页、GPX、PNG/JPEG、文件、mobileconfig等有SHA256；缺素材逐项BLOCKED
- 对照：`R-NONE`。

### ENV-006 无设备与无效目标诊断

- 适用：`HOST`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 仅在拔除全部手机后执行 list；再以不存在的显式UDID执行 info`
- 检查：list为空且结构合法；显式未知目标失败有限且不会选其它设备；结束后恢复连接
- 对照：`R-NONE`。

### ENV-007 JSON、人类输出与错误类别

- 适用：`HOST`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 对 info 分别默认/--no-json；对无效参数加 --error-format json`
- 检查：结构可解析、错误在stderr、非法参数与运行错误区分；无凭据泄漏；不以字段顺序差异判失败
- 对照：`R-NONE`。


<a id="disc"></a>
## DISC 发现、身份和主机/目标诊断

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-DISCOVERY`，详见 UPSTREAM.md；覆盖：`list`, `listen`, `discover`, `doctor`。

### DISC-001 USB单机/双机枚举

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos list`
- 检查：D1/D2的UDID和USB类型对应正确；单机/双机/空三种结果分别保存
- 对照：`R-DISCOVERY`。

### DISC-002 连接与拔出事件

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `40s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos listen`
- 检查：人工插拔单台；attach/detach对应正确、无串台；30秒后Ctrl-C结束
- 对照：`R-DISCOVERY`。

### DISC-003 mobdev2发现

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U discover mobdev2 --timeout 10`
- 检查：记录真实发现结果/协议身份；服务没有广告时不能伪报在线；未发现须与上游及网络条件对照
- 对照：`R-DISCOVERY`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；Wi-Fi相关用例需同LAN及明确已有网络信任

### DISC-004 mdns发现

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U discover mdns --timeout 10`
- 检查：记录真实发现结果/协议身份；服务没有广告时不能伪报在线；未发现须与上游及网络条件对照
- 对照：`R-DISCOVERY`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；Wi-Fi相关用例需同LAN及明确已有网络信任

### DISC-005 remotepairing发现

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U discover remotepairing --timeout 10`
- 检查：记录真实发现结果/协议身份；服务没有广告时不能伪报在线；未发现须与上游及网络条件对照
- 对照：`R-DISCOVERY`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；Wi-Fi相关用例需同LAN及明确已有网络信任

### DISC-006 paired-identity发现

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U discover paired-identity --timeout 10`
- 检查：记录真实发现结果/协议身份；服务没有广告时不能伪报在线；未发现须与上游及网络条件对照
- 对照：`R-DISCOVERY`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；Wi-Fi相关用例需同LAN及明确已有网络信任

### DISC-007 只读主机Doctor

- 适用：`HOST`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos doctor --timeout 3 --mdns`
- 检查：报告依赖、路由、usbmux/manager；Windows不适用macOS remoted；不自动创建配对/隧道
- 对照：`R-DISCOVERY`。

### DISC-008 Doctor绑定单一目标

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U doctor --timeout 3 --mdns`
- 检查：型号/OS/build/DDI证据来自所选设备；双机同时在线也不混用目标
- 对照：`R-DISCOVERY`。

### DISC-009 锁定/未信任/开发服务未就绪报告

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 对已登记的锁定和服务关闭状态重复 doctor/info`
- 检查：状态为具体缺前置/未知并有可执行建议，不能把本地文件存在等同设备可用；记录和上游的相同限制
- 对照：`R-DISCOVERY`。

### DISC-010 USB与Wi-Fi同机身份去重

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 两机在线时分别记录 list/discover；再拔掉目标USB复测`
- 检查：同一设备多传输身份可关联；Wi-Fi试验确实走网络，不是USB仍接着造成假覆盖
- 对照：`R-DISCOVERY`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；已有网络配对；现场可辨识真实transport


<a id="info"></a>
## INFO 设备信息、Lockdown值与心跳

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-INFO`，详见 UPSTREAM.md；覆盖：`info`, `lockdown`, `ip`, `batterycheck`, `batteryregistry`, `diskspace`, `mobilegestalt`, `heartbeat`。

### INFO-001 设备概要

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U info`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-002 Lockdown概要

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U info lockdown`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-003 传统显示信息

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U info display`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-004 根值查询

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown info`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-005 必要单字段

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown get --key ProductVersion`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-006 电量概要

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U batterycheck`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-007 电池IORegistry

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U batteryregistry`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-008 磁盘空间

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diskspace`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-009 MobileGestalt多键

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U mobilegestalt ProductType ProductVersion`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-010 设备时钟

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown date`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-011 语言

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown language`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-012 开发者模式

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown developer-mode`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-013 字号

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown font-size`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-014 时区

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown time-zone`
- 检查：读取对应实际字段；与手机设置/上游同字段语义一致，保留缺字段/拒绝/单位；不要求序列化完全相同
- 对照：`R-INFO`。

### INFO-015 CoreDevice lock-state

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U info lock-state`
- 检查：所选设备的实际CoreDevice值；服务不存在时按版本/上游分类
- 对照：`R-INFO`。

### INFO-016 CoreDevice device-info

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U info device-info`
- 检查：所选设备的实际CoreDevice值；服务不存在时按版本/上游分类
- 对照：`R-INFO`。

### INFO-017 CoreDevice display --coredevice

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U info display --coredevice`
- 检查：所选设备的实际CoreDevice值；服务不存在时按版本/上游分类
- 对照：`R-INFO`。

### INFO-018 查询 device-name

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown device-name`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-019 查询 locale

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown locale`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-020 查询 assistive-touch

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown assistive-touch`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-021 查询 voice-over

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown voice-over`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-022 查询 zoom-touch

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown zoom-touch`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-023 查询 invert-display

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown invert-display`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-024 查询 time-format

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown time-format`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-025 查询 wifi-connections

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown wifi-connections`
- 检查：查询值与实际设置对应；不得把查询当已批准写入
- 对照：`R-INFO`。

### INFO-026 Lockdown标准心跳

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `40s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown heartbeat`
- 检查：收到真实响应；外层30秒内人工Ctrl-C；无忙循环/悬挂
- 对照：`R-INFO`。

### INFO-027 有限心跳与计数

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U heartbeat --count 3 --timeout 15`
- 检查：收到3个真实心跳后结束；计数、间隔和退出正确，超时不能冒充收齐
- 对照：`R-INFO`。

### INFO-028 IP抓包推断

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `20s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ip --timeout 10`
- 检查：使用测试浏览器产生网络流量后输出目标实际IP；未抓到包不能凭空推断
- 对照：`R-INFO`。

### INFO-029 可逆设备名写入/读取/还原

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown device-name "IOS-LAB-TEST"`
- 检查：先保存原名；写入后手机和读取一致，再还原；上游只读回查
- 对照：`R-INFO`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；允许临时改设备名
- 特定清理：用原值还原device-name并回读确认

### INFO-030 Lockdown typed set/remove

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 仅对专门允许且可还原的测试域执行 lockdown set <VALUE> --key <KEY> --domain <DOMAIN> --type <TYPE>；remove --key <KEY> --domain <DOMAIN>`
- 检查：分别string/bool/int/float；回读类型和值正确；不存在域被系统拒绝可作上游对照，不能随意修改系统键
- 对照：`R-INFO`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；明确可写测试域/键和原值；无此素材BLOCKED

### INFO-031 可逆辅助设置写入

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 分别对 assistive-touch/voice-over/zoom-touch/invert-display/time-format/locale/wifi-connections 提供经--help确认的值再查询`
- 检查：每个参数单独subcheck；真实设备变化后还原原值；locale/Wi-Fi可能断连，独立记录
- 对照：`R-INFO`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；允许修改这些具体设置
- 特定清理：逐个还原原值，确认网络和辅助功能正常

### INFO-032 辅助功能重置

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown reset-accessibility`
- 检查：只有明确批准重置辅助设置时运行；原配置可恢复，否则NOT_RUN
- 对照：`R-INFO`。

### INFO-033 配对记录私密导出

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown save-pair-record "$PrivateRoot/pair-record.plist"`
- 检查：文件可供既有信任SDK验证；仅留大小/hash，私钥不能进CSV/Git/问题包
- 对照：`R-PAIR`。
- 特定清理：保存到用户控制的私密目录，按授权保留/销毁


<a id="pair"></a>
## PAIR 已有信任与显式配对

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-PAIR`，详见 UPSTREAM.md；覆盖：`pair`。

### PAIR-001 列出已有远程凭证

- 适用：`HOST`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pair --list`
- 检查：只查看既有凭证；不把 pair 无子命令误当list；不输出凭据内容
- 对照：`R-PAIR`。

### PAIR-002 Lockdown已有配对摘要

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pair show-record`
- 检查：摘要对应目标；敏感pair信息不进入共享报告
- 对照：`R-PAIR`。

### PAIR-003 显式RemotePairing

- 适用：`MODERN`；优先级 `P1`；权限 `TRUST_CHANGE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pair --address <OBSERVED_IPV6>`
- 检查：经用户批准后完成目标配对、手机信任提示与凭证保存；拒绝提示时有限结束
- 对照：`R-PAIR`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；允许新增信任；地址已人工对应设备

### PAIR-004 Wi-Fi RemotePairing

- 适用：`MODERN`；优先级 `P1`；权限 `TRUST_CHANGE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pair wifi --address <OBSERVED_IPV6>`
- 检查：目标身份准确，记录可用性及拒绝/重试；不能因另一手机成功而通过
- 对照：`R-PAIR`。

### PAIR-005 监督证书配对

- 适用：`BOTH`；优先级 `P1`；权限 `TRUST_CHANGE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pair supervised --p12 "$SupervisorP12"`
- 检查：仅有真实监督证书及受监督设备运行；否则BLOCKED；不自建监督状态
- 对照：`R-PAIR`。


<a id="ddi"></a>
## DDI 开发镜像、个性化与Cryptex

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-DDI`，详见 UPSTREAM.md；覆盖：`ddi`。

### DDI-001 status

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi status`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-002 list

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi list`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-003 devices

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi devices`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-004 lookup Developer

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi lookup Developer`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-005 devmode-status

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi devmode-status`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-006 nonce

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi nonce`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-007 personalization-identifiers

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi personalization-identifiers`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-008 personalization-manifests

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi personalization-manifests`
- 检查：读取实际镜像/硬件/nonce或manifest；传统与个性化路径按OS区分；本地digest匹配不代表Apple签名验证
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；个性化类子命令按设备版本探测，不适用需记录依据

### DDI-009 自动选图并挂载

- 适用：`BOTH`；优先级 `P1`；权限 `DDI_CHANGE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi auto`
- 检查：传统设备选major/minor匹配DDI；现代按硬件identity/所需服务；挂载后开发服务实际可用
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；合法本地DDI/个性化素材及在线签名需求许可

### DDI-010 指定镜像挂载/卸载

- 适用：`BOTH`；优先级 `P1`；权限 `DDI_CHANGE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi mount --path "$DdiRoot"`
- 检查：先记挂载基线；挂载并运行一次开发只读查询，再ddi unmount；恢复原状态
- 对照：`R-DDI`。

### DDI-011 身份不符、缺组件和digest错误

- 适用：`BOTH`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 在私有副本中分别使用不匹配/缺文件/破坏digest的DDI目录；先Doctor观察，禁止强制挂载`
- 检查：分别报告mismatch/incomplete/digest问题；不尝试签名绕过；保留具体fixture哈希
- 对照：`R-DDI`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；只改本地fixture副本

### DDI-012 Cryptex list

- 适用：`IOS27`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi cryptex list`
- 检查：先探测系统/服务；要求的Cryptex profile未满足时N/A/UNSUPPORTED，不能替代普通DDI验收
- 对照：`R-DDI`。

### DDI-013 Cryptex personalization-identifiers

- 适用：`IOS27`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi cryptex personalization-identifiers`
- 检查：先探测系统/服务；要求的Cryptex profile未满足时N/A/UNSUPPORTED，不能替代普通DDI验收
- 对照：`R-DDI`。

### DDI-014 Cryptex nonce

- 适用：`IOS27`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U ddi cryptex nonce`
- 检查：先探测系统/服务；要求的Cryptex profile未满足时N/A/UNSUPPORTED，不能替代普通DDI验收
- 对照：`R-DDI`。

### DDI-015 Cryptex安装/卸载

- 适用：`IOS27`；优先级 `P1`；权限 `DDI_CHANGE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: ddi cryptex auto-install；对本次实际identifier执行uninstall`
- 检查：需该系统支持、真实合法镜像和单独授权；安装/开发查询/卸载与基线还原分别记录
- 对照：`R-DDI`。


<a id="tun"></a>
## TUN 现代隧道、RSD、管理与权限

前置：iOS>=17；细分17.0–17.3/17.4+与实际服务；已有信任，不能默认新配对。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-TUNNEL`，详见 UPSTREAM.md；覆盖：`tunnel`, `rsd`。

### TUN-001 Userspace建立及终止

- 适用：`MODERN`；优先级 `P0`；权限 `READ`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U tunnel start --userspace`
- 检查：实际RSD地址/端口、选定设备匹配；TCP服务可查询；Ctrl-C后原进程退出且资源回收
- 对照：`R-TUNNEL`。

### TUN-002 Kernel建立及路由回收

- 适用：`MODERN`；优先级 `P1`；权限 `NETWORK_ADMIN`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U tunnel start`
- 检查：管理员与Wintun具备时真实接口/IPv6可达；结束后本次接口/路由回收，保留测试前后快照
- 对照：`R-TUNNEL`。
- 附加前置：iOS>=17；细分17.0–17.3/17.4+与实际服务；已有信任，不能默认新配对；Windows管理员、可用wintun.dll；不以Userspace结果代替

### TUN-003 Lockdown start-tunnel入口

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U lockdown start-tunnel`
- 检查：按当前版本协商真实隧道，退出后收尾；与显式tunnel route区分
- 对照：`R-TUNNEL`。

### TUN-004 RSD全部服务与能力

- 适用：`MODERN`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U rsd services --all --features`
- 检查：实际service名、port与features可追溯；空features不是deny-all；未实现访问不标绿
- 对照：`R-TUNNEL`。

### TUN-005 RSD服务检查

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U rsd check <OBSERVED_SERVICE>`
- 检查：分别检查实际存在与不存在的名字，结果对应同一设备目录
- 对照：`R-TUNNEL`。

### TUN-006 已有RSD只读观察

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U rsd observe --address "<OBSERVED_IPV6_WITH_PORT>"`
- 检查：沿现有明确endpoint观察；不隐式新建隧道/改变设备；保存目录对应证据
- 对照：`R-TUNNEL`。

### TUN-007 Manager列出与查询

- 适用：`MODERN`；优先级 `P0`；权限 `READ`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: TUNNEL_HTTP；tunnel serve --userspace；GET /tunnels 和 /tunnel/<UDID>`
- 检查：HTTP实际记录与存活设备对应；metadata不等于服务可达；manager独立占端口
- 对照：`R-TUNNEL`。

### TUN-008 Manager显式start/stop/list

- 适用：`MODERN`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: TUNNEL_HTTP 的 POST /start-tunnel、DELETE /tunnel/<UDID>；tunnel list/stop先核对--help`
- 检查：只启动/停止本次指定目标；返回值与实际连接、服务/进程寿命一致；不得杀旧会话所有隧道
- 对照：`R-TUNNEL`。

### TUN-009 双机隔离与旧系统拒绝

- 适用：`DUAL`；优先级 `P0`；权限 `READ`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: DUAL；旧机保持info/syslog，新机启动/停止tunnel`
- 检查：旧机基本查询不被阻塞；现代服务不能错误选旧机。仅一台>=17，双有效现代tunnel验收BLOCKED，不能凑数
- 对照：`R-TUNNEL`。

### TUN-010 断线恢复与不重放写操作

- 适用：`MODERN`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 连续只读RSD查询中人工拔插目标，观察有限退避及再次查询`
- 检查：已停目标不自动复活；旧资源退出；只读可恢复，install/HID等提交写入不被自动重放
- 对照：`R-TUNNEL`。

### TUN-011 Manager授权文件生成/检查

- 适用：`HOST`；优先级 `P1`；权限 `LOCAL_SECRET`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: AUTH；tunnel auth create/check --config <FILE> --token-file <FILE> --permission <PERMISSION>`
- 检查：list/connect/manage-tunnels/manage-server/debug逐项；错误token/设备/过期权限拒绝；token不写报告
- 对照：`R-TUNNEL`。

### TUN-012 HTTP授权和Host/Origin

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: AUTH_HTTP；授权/未授权token、错误Host/Origin、非loopback绑定分别请求`
- 检查：拒绝者不能建立设备连接或枚举越权数据；既有正确token正常；逐条保留HTTP状态/清理
- 对照：`R-TUNNEL`。

### TUN-013 Manager关闭与重复启动

- 适用：`MODERN`；优先级 `P0`；权限 `SESSION_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 对本次manager Ctrl-C、确认端口释放，再启动一次`
- 检查：两次均有限退出；本次子任务/forward已关闭，不残留接口/监听器
- 对照：`R-TUNNEL`。


<a id="app"></a>
## APP 应用、安装、进程控制

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-APPS`，详见 UPSTREAM.md；覆盖：`apps`。

### APP-001 列出应用 user

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps list --app-type user`
- 检查：实际所选设备应用分类与上游语义一致；空集合可合法，不能凭空补条目
- 对照：`R-APPS`。

### APP-002 列出应用 system

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps list --app-type system`
- 检查：实际所选设备应用分类与上游语义一致；空集合可合法，不能凭空补条目
- 对照：`R-APPS`。

### APP-003 列出应用 hidden

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps list --app-type hidden`
- 检查：实际所选设备应用分类与上游语义一致；空集合可合法，不能凭空补条目
- 对照：`R-APPS`。

### APP-004 列出应用 all

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps list --app-type all`
- 检查：实际所选设备应用分类与上游语义一致；空集合可合法，不能凭空补条目
- 对照：`R-APPS`。

### APP-005 列出应用 file-sharing

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps list --app-type file-sharing`
- 检查：实际所选设备应用分类与上游语义一致；空集合可合法，不能凭空补条目
- 对照：`R-APPS`。

### APP-006 详情

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps show "$TestBundle"`
- 检查：已知测试app的信息/图标实际正确，文件格式可读；无此app应准确not-found
- 对照：`R-APPS`。

### APP-007 图标

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps icons "$TestBundle" --output-dir "$CaseDir/icons"`
- 检查：已知测试app的信息/图标实际正确，文件格式可读；无此app应准确not-found
- 对照：`R-APPS`。

### APP-008 显式CoreDevice应用表

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps list --coredevice`
- 检查：真实CoreDevice应用结果；与默认/legacy路由区分
- 对照：`R-APPS`。

### APP-009 processes

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps processes`
- 检查：实际进程/root/install record响应对应所选设备；不可拿mock DBSequence作为wire证明
- 对照：`R-APPS`。

### APP-010 roots

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps roots`
- 检查：实际进程/root/install record响应对应所选设备；不可拿mock DBSequence作为wire证明
- 对照：`R-APPS`。

### APP-011 install-record "$TestBundle"

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps install-record "$TestBundle"`
- 检查：实际进程/root/install record响应对应所选设备；不可拿mock DBSequence作为wire证明
- 对照：`R-APPS`。

### APP-012 普通IPA安装

- 适用：`BOTH`；优先级 `P0`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps install "$IpaV1"`
- 检查：签名合法测试app实际出现在手机且可启动；有真实进度/Complete，退出0不足以单独通过
- 对照：`R-APPS`。

### APP-013 Streaming ZIP安装

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps install "$IpaV1" --streaming`
- 检查：同一IPA真实传输并安装；文件hash、进度和安装结果正确；与普通安装分开记录
- 对照：`R-APPS`。

### APP-014 升级保留测试数据

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps upgrade "$IpaV2"`
- 检查：同bundle版本由v1到v2、预置测试标记保留，失败不能假Complete
- 对照：`R-APPS`。

### APP-015 进程 launch "$TestBundle"

- 适用：`MODERN`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps launch "$TestBundle"`
- 检查：仅作用于本次测试进程；PID/启动效果/退出真实；monitor有限观察后Ctrl-C；pkill禁止通配所有应用
- 对照：`R-APPS`。

### APP-016 进程 monitor <TEST_PID>

- 适用：`MODERN`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps monitor <TEST_PID>`
- 检查：仅作用于本次测试进程；PID/启动效果/退出真实；monitor有限观察后Ctrl-C；pkill禁止通配所有应用
- 对照：`R-APPS`。

### APP-017 进程 kill <TEST_PID>

- 适用：`MODERN`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps kill <TEST_PID>`
- 检查：仅作用于本次测试进程；PID/启动效果/退出真实；monitor有限观察后Ctrl-C；pkill禁止通配所有应用
- 对照：`R-APPS`。

### APP-018 进程 signal <TEST_PID> <APPROVED_SIGNAL>

- 适用：`MODERN`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps signal <TEST_PID> <APPROVED_SIGNAL>`
- 检查：仅作用于本次测试进程；PID/启动效果/退出真实；monitor有限观察后Ctrl-C；pkill禁止通配所有应用
- 对照：`R-APPS`。

### APP-019 进程 pkill <EXACT_TEST_PROCESS_PATTERN>

- 适用：`MODERN`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps pkill <EXACT_TEST_PROCESS_PATTERN>`
- 检查：仅作用于本次测试进程；PID/启动效果/退出真实；monitor有限观察后Ctrl-C；pkill禁止通配所有应用
- 对照：`R-APPS`。

### APP-020 进程 spawn <APPROVED_TEST_EXECUTABLE>

- 适用：`MODERN`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps spawn <APPROVED_TEST_EXECUTABLE>`
- 检查：仅作用于本次测试进程；PID/启动效果/退出真实；monitor有限观察后Ctrl-C；pkill禁止通配所有应用
- 对照：`R-APPS`。

### APP-021 archive测试app

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps archive "$TestBundle"`
- 检查：应用归档/恢复的真实状态与测试数据可核对；不触及个人app
- 对照：`R-APPS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；测试app归档前标记、依赖状态已满足

### APP-022 restore测试app

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps restore "$TestBundle"`
- 检查：应用归档/恢复的真实状态与测试数据可核对；不触及个人app
- 对照：`R-APPS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；测试app归档前标记、依赖状态已满足

### APP-023 安装取消和再次操作

- 适用：`BOTH`；优先级 `P0`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 已批准测试IPA安装中分别Ctrl-C/拔线；恢复后查app和再次有限list`
- 检查：记录已提交后的实际状态未知/部分安装，不宣称回滚；临时文件与原服务清理可观察；不自动重试写入
- 对照：`R-APPS`。

### APP-024 无效/签名失效IPA

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps install "$InvalidTestIpa"`
- 检查：明确安装失败，原测试app不被误删；同一IPA与上游比较；无效ZIP预检不是物理安装成功
- 对照：`R-APPS`。

### APP-025 卸载测试app

- 适用：`BOTH`；优先级 `P0`；权限 `TEST_APP_WRITE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps uninstall "$TestBundle"`
- 检查：卸载后查无该测试app，其他app不变；只能卸载本次允许的bundle
- 对照：`R-APPS`。

### APP-026 未打包app目录安装

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `360s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps install "$UnpackedTestApp"`
- 检查：合法签名.app目录实际安装及可启动；目录/文件模式与同bundle IPA安装分别记录
- 对照：`R-APPS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；已批准未打包.app目录及内容hash

### APP-027 图标尺寸/placeholder/覆盖变体

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: apps icons "$TestBundle" --width <W> --height <H> --scale <S> --output-dir <NEW_DIR>；已存在输出分别无force/--force；无图标目标--no-placeholder`
- 检查：每个变体实测尺寸、占位策略和已有文件保护；输入非法值在访问前明确拒绝；不使用个人app作破坏性素材
- 对照：`R-APPS`。

### APP-028 InstallCoordination原wire和同client恢复

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U apps install-record "$TestBundle" --observe-zero-budget-recovery --rsd-address "<OBSERVED_IPV6_WITH_PORT>"`
- 检查：原DBSequence整数wire类型及零预算无发送→同一client合法查询恢复真实观察；不存在观察route/服务BLOCKED，不拿普通list替代
- 对照：`R-APPS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；明确已有RSD endpoint；如userspace代理用其真实--rsd-proxy-port


<a id="afc"></a>
## AFC AFC、HouseArrest与CoreDevice文件

前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录。
清理：仅删除本次manifest里明确创建的文件/目录；保留其它原文件，不能rm根目录。
对照索引：`R-FILE`，详见 UPSTREAM.md；覆盖：`file`。

### AFC-001 根目录枚举

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U file ls /`
- 检查：目录/大小/类型与实际测试fixture一致；无权目录与不存在文件区分
- 对照：`R-FILE`。

### AFC-002 设备文件空间

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U file device-info`
- 检查：目录/大小/类型与实际测试fixture一致；无权目录与不存在文件区分
- 对照：`R-FILE`。

### AFC-003 文件属性

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U file stat "$RemoteFile"`
- 检查：目录/大小/类型与实际测试fixture一致；无权目录与不存在文件区分
- 对照：`R-FILE`。

### AFC-004 目录树

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U file tree "$RemoteRoot"`
- 检查：目录/大小/类型与实际测试fixture一致；无权目录与不存在文件区分
- 对照：`R-FILE`。

### AFC-005 创建/上传/下载/校验

- 适用：`BOTH`；优先级 `P0`；权限 `TEST_FILE_WRITE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: file mkdir "$RemoteRoot"；file push "$LocalFixture" "$RemoteFile"；file pull "$RemoteFile" "$CaseDir/roundtrip.bin"`
- 检查：上传下载SHA256与字节数一致；零字节、中文/空格、1MiB和64MiB分别subcheck
- 对照：`R-FILE`。

### AFC-006 移动重命名/删除

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_FILE_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: file mv <OWNED_FROM> <OWNED_TO>；file rm <OWNED_FILE>；最后file rm <OWNED_DIR> -r`
- 检查：移动后旧路径不存在/新hash相同；删除仅测试路径；清理后root基线无无关变化
- 对照：`R-FILE`。

### AFC-007 App完整容器

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U file --app "$TestBundle" ls /`
- 检查：对应指定app沙盒；非debuggable/file-sharing权限限制与上游对照
- 对照：`R-FILE`。

### AFC-008 Documents容器往返

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_FILE_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: file --app "$TestBundle" --documents 的 ls/push/pull/stat/mv/rm`
- 检查：只在测试app Documents独立目录完成往返，实际file sharing和容器权限明确
- 对照：`R-FILE`。

### AFC-009 Crash AFC只读

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U file --crash ls /`
- 检查：内容对应crash服务域，不混淆普通AFC根
- 对照：`R-FILE`。

### AFC-010 CoreDevice目录与读写

- 适用：`MODERN`；优先级 `P1`；权限 `TEST_FILE_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: file --coredevice --domain <OBSERVED_DOMAIN> --identifier <TEST_ID> 的 ls/push/pull/mkdir/mv/rm`
- 检查：已授权真实域内分项执行；服务域/路径/数据正确，不能拿AFC结果代替CoreDevice；device-info/stat/tree不在该模式支持范围
- 对照：`R-FILE`。

### AFC-011 Windows下载 new

- 适用：`BOTH`；优先级 `P0`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC new`
- 检查：新目标正确创建且hash匹配；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-012 Windows下载 existing-no-force

- 适用：`BOTH`；优先级 `P0`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC existing-no-force`
- 检查：已有目标无--force被拒绝且原hash不变；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-013 Windows下载 force

- 适用：`BOTH`；优先级 `P0`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC force`
- 检查：--force成功替换且hash匹配；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-014 Windows下载 unicode

- 适用：`BOTH`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC unicode`
- 检查：中文/空格/emoji位置与hash正确；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-015 Windows下载 long-path

- 适用：`BOTH`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC long-path`
- 检查：普通绝对路径超过260 UTF-16单位实际成功或具体OS限制；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-016 Windows下载 verbatim

- 适用：`BOTH`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC verbatim`
- 检查：显式\\?\路径位置/字节正确；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-017 Windows下载 readonly

- 适用：`BOTH`；优先级 `P0`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC readonly`
- 检查：只读文件/目录失败后原hash/marker不变且无临时残留；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-018 Windows下载 recovery

- 适用：`BOTH`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC recovery`
- 检查：前次失败后正常下载成功，无旧状态污染；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-019 Windows下载 unc

- 适用：`BOTH`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: WINDOWS_AFC unc`
- 检查：用户明确提供的真实UNC共享路径读写权限有效，下载hash正确；每项独立证据，不以另一个路径形式代替
- 对照：`R-FILE`。
- 附加前置：先确认AFC测试根目录/测试app容器；任何写入只在本次专属目录；unc需实际授权共享；长路径需记录Windows设置

### AFC-020 读写中断与后续恢复

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_FILE_WRITE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 测试大文件传输中Ctrl-C/拔线，再以新连接拉取已知文件`
- 检查：不报告完整成功；部分本地产物明确标记；已有目标保护、设备文件状态可核对；后续合法读无迟到回复
- 对照：`R-FILE`。

### AFC-021 CoreDevice未支持操作拒绝

- 适用：`MODERN`；优先级 `P1`；权限 `KNOWN_GAP`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: file --coredevice --domain <OBSERVED_DOMAIN> --identifier <TEST_ID> 的 device-info/stat/tree 分别核对拒绝`
- 检查：当前CoreDevice模式明确只支持ls/pull/push/rm/mkdir/mv；这三项记录UNSUPPORTED，不能把AFC同名成功代替它
- 对照：`R-FILE`。


<a id="back"></a>
## BACK MobileBackup2与本地备份工具

前置：仅批准测试数据/备份目录；备份可能包含私密信息，原文只进private目录。
清理：关闭本次服务，保留备份做后续local用例；密码不入命令记录/问题包。
对照索引：`R-BACKUP`，详见 UPSTREAM.md；覆盖：`backup`。

### BACK-001 version

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup version`
- 检查：真实协议/加密状态可核对，不改变密码/设备加密配置
- 对照：`R-BACKUP`。

### BACK-002 encryption-status

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup encryption-status`
- 检查：真实协议/加密状态可核对，不改变密码/设备加密配置
- 对照：`R-BACKUP`。

### BACK-003 完整备份

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `1800s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup create "$BackupDir" --full`
- 检查：生成完整备份及Manifest；有设备完成终态、文件hash/条目可读；非空目录不是成功证明
- 对照：`R-BACKUP`。

### BACK-004 增量备份

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `1800s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup create "$BackupDir"`
- 检查：修改专属测试app标记后增量备份；更新条目可核对，不把full fallback冒充增量
- 对照：`R-BACKUP`。

### BACK-005 过滤备份与manifest修剪

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `1800s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: backup create <NEW_DIR> --only <APPROVED_PRESET> / --only-regex <REGEX>；副本测试--patch-manifest/--unback`
- 检查：过滤前后条目/文件一致、其他原备份不被改；各flag单独subcheck，不能仅看退出码
- 对照：`R-BACKUP`。

### BACK-006 info

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup info "$BackupDir"`
- 检查：真实完成备份上的metadata/条目/解包结果正确；list-local/unback需确认不访问设备
- 对照：`R-BACKUP`。

### BACK-007 list

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup list "$BackupDir"`
- 检查：真实完成备份上的metadata/条目/解包结果正确；list-local/unback需确认不访问设备
- 对照：`R-BACKUP`。

### BACK-008 list-local

- 适用：`HOST`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup list-local "$BackupDir"`
- 检查：真实完成备份上的metadata/条目/解包结果正确；list-local/unback需确认不访问设备
- 对照：`R-BACKUP`。

### BACK-009 unback

- 适用：`HOST`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup unback "$BackupDir"`
- 检查：真实完成备份上的metadata/条目/解包结果正确；list-local/unback需确认不访问设备
- 对照：`R-BACKUP`。

### BACK-010 单文件本地提取

- 适用：`HOST`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup extract "$BackupDir" <TEST_DOMAIN> <TEST_RELATIVE_PATH> --output "$CaseDir/extract"`
- 检查：内容与设备预置fixture/hash一致；加密备份正确密码/错密码分开；host-only不能算服务传输
- 对照：`R-BACKUP`。

### BACK-011 unback-device

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: backup unback-device <BACKUP_DIR> [该子命令要求的domain/path]`
- 检查：由真实设备MobileBackup2执行；与host local变体分开记录；上游无等价记NO_COMPARATOR
- 对照：`R-BACKUP`。

### BACK-012 extract-device

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: backup extract-device <BACKUP_DIR> [该子命令要求的domain/path]`
- 检查：由真实设备MobileBackup2执行；与host local变体分开记录；上游无等价记NO_COMPARATOR
- 对照：`R-BACKUP`。

### BACK-013 加密开关和密码修改

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: backup encryption on/off；backup change-password（先按--help确认密码输入方式）`
- 检查：只有专属可恢复设备与已知旧密码可运行；分别验证备份加密和正确解密，再恢复原配置
- 对照：`R-BACKUP`。

### BACK-014 恢复已有备份

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `1800s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U backup restore "$BackupDir" --force`
- 检查：只有单独批准恢复整机数据后执行；完整性及终态需现场核对，禁止拿dry-run/信息查询代替
- 对照：`R-BACKUP`。


<a id="log"></a>
## LOG 日志、抓包、崩溃和符号

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：Ctrl-C或有限结束后确认流/文件关闭；原始日志/pcap/crash仅留私密证据。
对照索引：`R-LOGS`，详见 UPSTREAM.md；覆盖：`syslog`, `os-trace`, `pcap`, `btlogger`, `crash`, `file-relay`, `symbols`。

### LOG-001 Syslog真实事件

- 适用：`BOTH`；优先级 `P0`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U syslog --count 20 --timeout 30`
- 检查：在测试app产生带run_id标记的日志；收到对应手机/事件，达count或超时分类正确
- 对照：`R-LOGS`。

### LOG-002 OS trace ps

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U os-trace ps`
- 检查：进程/事件或归档可读且对应设备；trace格式/时钟有单位，不把文件存在视为有效完成
- 对照：`R-LOGS`。

### LOG-003 OS trace stream --count 20 --timeout 15

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U os-trace stream --count 20 --timeout 15`
- 检查：进程/事件或归档可读且对应设备；trace格式/时钟有单位，不把文件存在视为有效完成
- 对照：`R-LOGS`。

### LOG-004 OS trace archive "$CaseDir/archive.logarchive"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U os-trace archive "$CaseDir/archive.logarchive"`
- 检查：进程/事件或归档可读且对应设备；trace格式/时钟有单位，不把文件存在视为有效完成
- 对照：`R-LOGS`。

### LOG-005 OS trace collect "$CaseDir/collect.logarchive"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U os-trace collect "$CaseDir/collect.logarchive"`
- 检查：进程/事件或归档可读且对应设备；trace格式/时钟有单位，不把文件存在视为有效完成
- 对照：`R-LOGS`。

### LOG-006 IP包捕获

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pcap --duration 10 --output "$CaseDir/device.pcap"`
- 检查：制造测试流量后用Wireshark等解析包及时间戳/方向；零包不默认PASS
- 对照：`R-LOGS`。

### LOG-007 Bluetooth packetlogger

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U btlogger capture "$CaseDir/bt.pklg" --duration 10 --format packetlogger`
- 检查：产生可控蓝牙动作；对应格式可解析/时间戳有效；无蓝牙硬件/服务记BLOCKED
- 对照：`R-LOGS`。

### LOG-008 Bluetooth pcapng

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U btlogger capture "$CaseDir/bt.pcapng" --duration 10 --format pcapng`
- 检查：产生可控蓝牙动作；对应格式可解析/时间戳有效；无蓝牙硬件/服务记BLOCKED
- 对照：`R-LOGS`。

### LOG-009 Crash ls

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U crash ls`
- 检查：crash服务准备可能搬运全设备pending报告；flush为全设备pending flush，不是按测试app过滤。只读取/下载指定测试报告；真实内容/新事件与移动前后状态分别记录
- 对照：`R-LOGS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；用户允许CrashReportMover整理设备pending报告；无此许可NOT_RUN，不能假设只影响测试app

### LOG-010 Crash show <TEST_REPORT>

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U crash show <TEST_REPORT>`
- 检查：crash服务准备可能搬运全设备pending报告；flush为全设备pending flush，不是按测试app过滤。只读取/下载指定测试报告；真实内容/新事件与移动前后状态分别记录
- 对照：`R-LOGS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；用户允许CrashReportMover整理设备pending报告；无此许可NOT_RUN，不能假设只影响测试app

### LOG-011 Crash pull <TEST_REPORT> "$CaseDir/test.ips"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U crash pull <TEST_REPORT> "$CaseDir/test.ips"`
- 检查：crash服务准备可能搬运全设备pending报告；flush为全设备pending flush，不是按测试app过滤。只读取/下载指定测试报告；真实内容/新事件与移动前后状态分别记录
- 对照：`R-LOGS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；用户允许CrashReportMover整理设备pending报告；无此许可NOT_RUN，不能假设只影响测试app

### LOG-012 Crash pull-all <EXACT_TEST_PATTERN> "$CaseDir/crashes"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U crash pull-all <EXACT_TEST_PATTERN> "$CaseDir/crashes"`
- 检查：crash服务准备可能搬运全设备pending报告；flush为全设备pending flush，不是按测试app过滤。只读取/下载指定测试报告；真实内容/新事件与移动前后状态分别记录
- 对照：`R-LOGS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；用户允许CrashReportMover整理设备pending报告；无此许可NOT_RUN，不能假设只影响测试app

### LOG-013 Crash flush

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U crash flush`
- 检查：crash服务准备可能搬运全设备pending报告；flush为全设备pending flush，不是按测试app过滤。只读取/下载指定测试报告；真实内容/新事件与移动前后状态分别记录
- 对照：`R-LOGS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；用户允许CrashReportMover整理设备pending报告；无此许可NOT_RUN，不能假设只影响测试app

### LOG-014 Crash watch --timeout 20

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U crash watch --timeout 20`
- 检查：crash服务准备可能搬运全设备pending报告；flush为全设备pending flush，不是按测试app过滤。只读取/下载指定测试报告；真实内容/新事件与移动前后状态分别记录
- 对照：`R-LOGS`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；用户允许CrashReportMover整理设备pending报告；无此许可NOT_RUN，不能假设只影响测试app

### LOG-015 本地crash解析

- 适用：`HOST`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: crash parse <COPIED_TEST_IPS> --json；crash parse-latest <CASE_DIR>`
- 检查：进程、异常与frame信息可核对；损坏副本明确错误；parse-latest选中文件正确
- 对照：`R-LOGS`。

### LOG-016 Crash删除/清空

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: crash rm <EXACT_TEST_PATTERN> --force；crash clear --force只在批准全清设备运行`
- 检查：删除范围正确且不可自动对所有真实报告运行；rm默认*绝不能省略pattern
- 对照：`R-LOGS`。

### LOG-017 Crash sysdiagnose未实现边界

- 适用：`BOTH`；优先级 `P1`；权限 `KNOWN_GAP`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U crash sysdiagnose`
- 检查：记录当前明确未实现；不要触发真实恢复或当成成功诊断包
- 对照：`R-LOGS`。

### LOG-018 File Relay归档

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U file-relay <APPROVED_SOURCE> --output "$CaseDir/relay.gz"`
- 检查：合法来源真实归档可解；服务/来源拒绝与上游对应；不是任意系统目录读取
- 对照：`R-LOGS`。

### LOG-019 符号列表

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U symbols list`
- 检查：符号条目实际可用；传统/现代支持由设备判定
- 对照：`R-LOGS`。

### LOG-020 单符号与完整下载

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `600s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: symbols pull <OBSERVED_INDEX> <NEW_OUTPUT>；symbols download <NEW_DIRECTORY>`
- 检查：索引来自本机实际list；符号文件可解析/有size/hash；完整下载单独计数和限额
- 对照：`R-LOGS`。


<a id="diag"></a>
## DIAG 诊断、性能与调试

前置：开发者模式与对应DDI/开发服务可用；测试app/测试PID已登记。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-DEVELOPER`，详见 UPSTREAM.md；覆盖：`diagnostics`, `instruments`, `devicestate`, `debug`, `debugserver`, `accessibility-audit`, `memlimitoff`, `power-assert`。

### DIAG-001 Diagnostics battery

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics battery`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-002 Diagnostics battery-monitor --count 3

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics battery-monitor --count 3`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-003 Diagnostics list

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics list`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-004 Diagnostics show <OBSERVED_DIAGNOSTIC>

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics show <OBSERVED_DIAGNOSTIC>`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-005 Diagnostics gasgauge

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics gasgauge`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-006 Diagnostics hdmi

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics hdmi`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-007 Diagnostics wifi

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics wifi`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-008 Diagnostics nand

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics nand`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-009 Diagnostics ioregistry <OBSERVED_ENTRY_CLASS>

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics ioregistry <OBSERVED_ENTRY_CLASS>`
- 检查：返回真实设备诊断字段/单位；HDMI等硬件缺失不直接判缺陷；battery-monitor有有限上限
- 对照：`R-DEVELOPER`。

### DIAG-010 CoreDevice sysdiagnose预览

- 适用：`MODERN`；优先级 `P1`；权限 `KNOWN_GAP`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics sysdiagnose`
- 检查：当前预览/未实现返回明确；不将preview当真实诊断归档
- 对照：`R-DEVELOPER`。

### DIAG-011 Instruments ps

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments ps`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-012 Instruments sysattrs

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments sysattrs`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-013 Instruments procattrs

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments procattrs`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-014 Instruments apps

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments apps`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-015 Instruments cpu

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments cpu --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-016 Instruments energy --pid <TEST_PID>

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments energy --pid <TEST_PID> --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-017 Instruments network

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments network --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-018 Instruments gpu

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments gpu --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-019 Instruments trace

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments trace --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-020 Instruments har

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments har --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-021 Instruments fps

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments fps --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-022 Instruments kdebug

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments kdebug --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-023 Instruments sysmon-process

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments sysmon-process --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-024 Instruments sysmon-threshold 80

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments sysmon-threshold 80 --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-025 Instruments notifications

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments notifications --count 1`
- 检查：通过测试app CPU/网络/画面变化验证实际采样；单位/时间/目标PID正确；HAR/trace含敏感数据，只记摘要
- 对照：`R-DEVELOPER`。

### DIAG-026 Instruments launch "$TestBundle"

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments launch "$TestBundle"`
- 检查：实际启动/终止仅本次测试app；legacy/modern路由分别实测
- 对照：`R-DEVELOPER`。

### DIAG-027 Instruments kill <TEST_PID>

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U instruments kill <TEST_PID>`
- 检查：实际启动/终止仅本次测试app；legacy/modern路由分别实测
- 对照：`R-DEVELOPER`。

### DIAG-028 Device condition配置列表

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U devicestate list`
- 检查：仅列实际device condition类别/配置，不假设固定ID
- 对照：`R-DEVELOPER`。

### DIAG-029 模拟设备condition

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: devicestate enable <OBSERVED_TYPE_ID> <OBSERVED_PROFILE_ID>；最后disable`
- 检查：明确授权的温度/网络等测试condition真实生效且恢复；ID取本机list
- 对照：`R-DEVELOPER`。

### DIAG-030 有限电源保持

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U power-assert --timeout 10`
- 检查：观察10秒保持请求/有限退出；清理后恢复正常休眠行为
- 对照：`R-DEVELOPER`。

### DIAG-031 调试连接信息

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U debug "$TestBundle"`
- 检查：地址/服务/LLDB命令对应真实目标；输出命令不证明已调试成功
- 对照：`R-DEVELOPER`。

### DIAG-032 Debugserver安全包

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U debugserver send <APPROVED_READ_ONLY_RSP_PACKET>`
- 检查：已批准的读握手包获得实际RSP；未知包不发送；如真实LLDB可用补连接/关闭subcheck
- 对照：`R-DEVELOPER`。

### DIAG-033 进程内存限制

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U memlimitoff <TEST_PID>`
- 检查：只允许测试进程；权限拒绝先上游比对，不能对系统PID注入；终止测试进程恢复
- 对照：`R-DEVELOPER`。

### DIAG-034 Accessibility capabilities

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit capabilities`
- 检查：测试页面真实属性/审计结果；服务缺失或iOS限制先比对；不能以capability存在替代审计完成
- 对照：`R-DEVELOPER`。

### DIAG-035 Accessibility audit-types

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit audit-types`
- 检查：测试页面真实属性/审计结果；服务缺失或iOS限制先比对；不能以capability存在替代审计完成
- 对照：`R-DEVELOPER`。

### DIAG-036 Accessibility settings

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit settings`
- 检查：测试页面真实属性/审计结果；服务缺失或iOS限制先比对；不能以capability存在替代审计完成
- 对照：`R-DEVELOPER`。

### DIAG-037 Accessibility list-items

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit list-items`
- 检查：测试页面真实属性/审计结果；服务缺失或iOS限制先比对；不能以capability存在替代审计完成
- 对照：`R-DEVELOPER`。

### DIAG-038 Accessibility describe

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit describe`
- 检查：测试页面真实属性/审计结果；服务缺失或iOS限制先比对；不能以capability存在替代审计完成
- 对照：`R-DEVELOPER`。

### DIAG-039 Accessibility run-audit <OBSERVED_AUDIT_TYPE>

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit run-audit <OBSERVED_AUDIT_TYPE>`
- 检查：测试页面真实属性/审计结果；服务缺失或iOS限制先比对；不能以capability存在替代审计完成
- 对照：`R-DEVELOPER`。

### DIAG-040 Accessibility输入 navigate next

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit navigate next`
- 检查：在受控测试app验证焦点变化/点击效果，执行后回到基线
- 对照：`R-DEVELOPER`。

### DIAG-041 Accessibility输入 tap

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U accessibility-audit tap`
- 检查：在受控测试app验证焦点变化/点击效果，执行后回到基线
- 对照：`R-DEVELOPER`。

### DIAG-042 显式reboot

- 适用：`BOTH`；优先级 `P1`；权限 `REBOOT`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics reboot --force`
- 检查：人工批准、保存其它用例后执行；记录断开/重启/重连；shutdown需要人工开机
- 对照：`R-DEVELOPER`。

### DIAG-043 显式shutdown

- 适用：`BOTH`；优先级 `P1`；权限 `REBOOT`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U diagnostics shutdown --force`
- 检查：人工批准、保存其它用例后执行；记录断开/重启/重连；shutdown需要人工开机
- 对照：`R-DEVELOPER`。


<a id="ui"></a>
## UI 截图与SpringBoard

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-SCREEN`，详见 UPSTREAM.md；覆盖：`screenshot`, `springboard`。

### UI-001 真实屏幕截图

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U screenshot --output "$CaseDir/screen.png"`
- 检查：显示带run_id测试画面；PNG真实可读、尺寸/方向和画面对应；不能只检查文件头
- 对照：`R-SCREEN`。

### UI-002 截图覆盖保护

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 重复 screenshot --output 同一路径，不带/带 --force 分开`
- 检查：无force保留原文件，force生成新实际画面；旧hash与新hash均留存
- 对照：`R-SCREEN`。

### UI-003 SpringBoard icons

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard icons`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-004 SpringBoard state

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard state`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-005 SpringBoard orientation

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard orientation`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-006 SpringBoard icon-metrics

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard icon-metrics`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-007 SpringBoard find "$TestBundle"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard find "$TestBundle"`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-008 SpringBoard icon "$TestBundle" "$CaseDir/icon.png"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard icon "$TestBundle" "$CaseDir/icon.png"`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-009 SpringBoard icon-all "$CaseDir/icons"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard icon-all "$CaseDir/icons"`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-010 SpringBoard wallpaper-info <OBSERVED_WALLPAPER_NAME>

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard wallpaper-info <OBSERVED_WALLPAPER_NAME>`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-011 SpringBoard wallpaper-preview-image <OBSERVED_NAME> "$CaseDir/wallpaper-preview.png"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard wallpaper-preview-image <OBSERVED_NAME> "$CaseDir/wallpaper-preview.png"`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-012 SpringBoard wallpaper-home-screen "$CaseDir/wallpaper-home.png"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard wallpaper-home-screen "$CaseDir/wallpaper-home.png"`
- 检查：布局/方向/图标或壁纸实际内容对应；个人wallpaper只保存私密证据
- 对照：`R-SCREEN`。

### UI-013 SpringBoard布局写入/还原

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U springboard state-set "$ApprovedStatePlist"`
- 检查：先备份原布局，仅测试布局；真实变化后用原布局还原，否则BLOCKED
- 对照：`R-SCREEN`。

### UI-014 截图multipart流

- 适用：`BOTH`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U screenshot --stream --host 127.0.0.1 --port 3333`
- 检查：真实HTTP multipart多帧、连续至少10次测试画面变化，逐帧可读/尺寸正确；客户端断连和服务Ctrl-C后端口/进程清理；非loopback绑定拒绝
- 对照：`R-SCREEN`。


<a id="control"></a>
## CONTROL 现代设置、旋转和Display

前置：实际iOS>=17及服务；HID/Display可能需要kernel隧道与管理员/Wintun；先准备受控测试画面。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-COREDEVICE`，详见 UPSTREAM.md；覆盖：`device-control`, `hid`。

### CONTROL-001 查询配置 style

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get style`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-002 查询配置 color-filter

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get color-filter`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-003 查询配置 text-size

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get text-size`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-004 查询配置 reduce-motion

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get reduce-motion`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-005 查询配置 increase-contrast

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get increase-contrast`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-006 查询配置 show-borders

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get show-borders`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-007 查询配置 reduce-transparency

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get reduce-transparency`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-008 查询配置 liquid-glass-opacity

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control configuration get liquid-glass-opacity`
- 检查：原生值、类型与设备UI一致；系统不具备Liquid Glass等功能时准确分类，不假定所有17+支持
- 对照：`R-COREDEVICE`。

### CONTROL-009 设置配置后还原

- 适用：`MODERN`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 每个已支持setting分别 device-control configuration set <SETTING> <APPROVED_VALUE>，然后get`
- 检查：逐项验证style/颜色/字号/辅助开关变化与回读，最后恢复每项原值；不将8项合成一个无明细PASS
- 对照：`R-COREDEVICE`。

### CONTROL-010 真实旋转 left

- 适用：`MODERN`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control orientation left`
- 检查：手机真实90度变化且结果非空成功；配合截图/坐标复查，最后还原
- 对照：`R-COREDEVICE`。

### CONTROL-011 真实旋转 right

- 适用：`MODERN`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control orientation right`
- 检查：手机真实90度变化且结果非空成功；配合截图/坐标复查，最后还原
- 对照：`R-COREDEVICE`。

### CONTROL-012 Display能力及媒体状态

- 适用：`MODERN`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control display status`
- 检查：真实support/status，保留测试前baseline供媒体停止核对；只读status不证明启动/stop
- 对照：`R-COREDEVICE`。

### CONTROL-013 编码RTP video

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `30s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control display video --max-units 10 --timeout 15 --output "$CaseDir/capture.hevc"`
- 检查：得到真实编码单元、时间/大小，外部解码验证内容；停止后原服务回baseline，不把EOF当ACK
- 对照：`R-COREDEVICE`。

### CONTROL-014 编码RTP audio

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `30s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U device-control display audio --max-units 10 --timeout 15 --output "$CaseDir/capture.aac"`
- 检查：得到真实编码单元、时间/大小，外部解码验证内容；停止后原服务回baseline，不把EOF当ACK
- 对照：`R-COREDEVICE`。

### CONTROL-015 MediaInUse不误停其它流

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 在人工允许的第二个媒体客户端占用期间启动本项目媒体`
- 检查：若返回MediaInUse，本项目退出不能stopAll杀掉其它流；两端记录，结束后各自关闭
- 对照：`R-COREDEVICE`。

### CONTROL-016 HID硬件按钮press/release

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: hid --confirm button --usage-code <SAFE_USAGE> --state down；同code --state up`
- 检查：选择人工批准的非破坏按钮；真实效果且无按键卡住；usage码必须现场确认，不猜值
- 对照：`R-COREDEVICE`。

### CONTROL-017 归一化tap

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U hid --confirm tap --x 0.5 --y 0.5`
- 检查：只在测试app/网页验证真实点击/滚动/文本/按键；flush成功不是设备ACK；结束后释放键/媒体
- 对照：`R-COREDEVICE`。

### CONTROL-018 swipe

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U hid --confirm swipe --from-x 0.3 --from-y 0.5 --to-x 0.7 --to-y 0.5`
- 检查：只在测试app/网页验证真实点击/滚动/文本/按键；flush成功不是设备ACK；结束后释放键/媒体
- 对照：`R-COREDEVICE`。

### CONTROL-019 文字输入

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U hid --confirm text "ios-lab-中文-123"`
- 检查：只在测试app/网页验证真实点击/滚动/文本/按键；flush成功不是设备ACK；结束后释放键/媒体
- 对照：`R-COREDEVICE`。

### CONTROL-020 键盘修饰键

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U hid --confirm key <APPROVED_USAGE> --shift`
- 检查：只在测试app/网页验证真实点击/滚动/文本/按键；flush成功不是设备ACK；结束后释放键/媒体
- 对照：`R-COREDEVICE`。

### CONTROL-021 显式touch phases

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: hid --confirm touch down/move/up --contact-id 0 --x <X> --y <Y>（先按--help确认phase）`
- 检查：逐相位真实触摸与最终release；不在非测试app输入；不可观察/取消中release失败保留原错误
- 对照：`R-COREDEVICE`。

### CONTROL-022 HID停止观察与清理

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U hid --confirm observe-stop --expected-status-sha256 <APPROVED_BASELINE_HASH> --confirm-stop-all`
- 检查：仅在批准服务级stopAll且有真实baseline时运行；停止/原生任务收尾、再次status一致
- 对照：`R-COREDEVICE`。
- 附加前置：实际iOS>=17及服务；HID/Display可能需要kernel隧道与管理员/Wintun；先准备受控测试画面；协议观察baseline，不能拿任意JSON hash冒充原生编码hash


<a id="clip"></a>
## CLIP 设备剪贴板与模拟定位

前置：先用专属测试文本/图片替换个人剪贴板并记录可恢复基线。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-CLIPBOARD`，详见 UPSTREAM.md；覆盖：`pasteboard`, `location`。

### CLIP-001 设备文本读取

- 适用：`MODERN`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pasteboard get`
- 检查：设备测试文本读回一致；默认脱敏输出的占位不能当数据损坏；按--help和许可选show-data
- 对照：`R-CLIPBOARD`。

### CLIP-002 写入并在设备粘贴

- 适用：`MODERN`；优先级 `P0`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pasteboard set "ios-lab-中文-123"`
- 检查：在测试app实际粘贴与字节一致，读取和UI效果都验证；最后恢复测试基线
- 对照：`R-CLIPBOARD`。

### CLIP-003 watch、去重和回环

- 适用：`MODERN`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pasteboard watch`
- 检查：在设备修改测试内容；同内容不重复，变更顺序正确；监听30秒后Ctrl-C有界退出
- 对照：`R-CLIPBOARD`。

### CLIP-004 watch断线恢复

- 适用：`MODERN`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: pasteboard watch；人工拔插/网络恢复，再改变测试内容`
- 检查：有界重连与实际fresh pull；不重放SET，不输出历史私密值；Ctrl-C停止不复活
- 对照：`R-CLIPBOARD`。

### CLIP-005 实验 resolve <OBSERVED_INDEX> <OBSERVED_UTI> --experimental

- 适用：`MODERN`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pasteboard resolve <OBSERVED_INDEX> <OBSERVED_UTI> --experimental`
- 检查：只对本次测试数据的真实item/UTI；不支持/无上游等价要如实记录，不能凭返回0当已成功导出图片
- 对照：`R-CLIPBOARD`。

### CLIP-006 实验 export <OBSERVED_INDEX> <OBSERVED_UTI> "$CaseDir/clipboard.bin" --experimental

- 适用：`MODERN`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pasteboard export <OBSERVED_INDEX> <OBSERVED_UTI> "$CaseDir/clipboard.bin" --experimental`
- 检查：只对本次测试数据的真实item/UTI；不支持/无上游等价要如实记录，不能凭返回0当已成功导出图片
- 对照：`R-CLIPBOARD`。

### CLIP-007 实验autonotify

- 适用：`MODERN`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U pasteboard watch --autonotify --experimental`
- 检查：真实设备通知及fresh数据，去重/有限取消；缺原生支持应显式结果，不升级成熟度
- 对照：`R-CLIPBOARD`。

### CLIP-008 单点定位及reset

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: location set <APPROVED_LAT> <APPROVED_LON>；location reset`
- 检查：地图/测试app实际位置变化；reset后恢复真实位置；legacy也执行本项
- 对照：`R-CLIPBOARD`。

### CLIP-009 GPX轨迹及取消

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U location gpx "$GpxFile"`
- 检查：测试app看到多点/间隔，Ctrl-C结束后显式reset；取消写/断连不报告完整走完
- 对照：`R-CLIPBOARD`。

### CLIP-010 raw/policy剪贴板读取

- 适用：`MODERN`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: pasteboard get --raw --policy <resolved|promised|matchsource|promisesecondary|threshold:N>；只对本次测试数据按许可--show-data`
- 检查：逐policy/多表示UTI/size/hash真实可核对；show-data是敏感显式输出，不把默认脱敏摘要当数据丢失
- 对照：`R-CLIPBOARD`。

### CLIP-011 URL及多UTI表示写入

- 适用：`MODERN`；优先级 `P1`；权限 `CLIPBOARD_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: pasteboard set --url <LAB_URL>；pasteboard set <TEST_TEXT> --uti <OBSERVED_TEXT_UTI>`
- 检查：测试app实际粘贴/读取URL与不同表示，不能只验证普通文本；还原原测试基线
- 对照：`R-CLIPBOARD`。

### CLIP-012 直接设备PNG/JPEG原始UTI

- 适用：`MODERN`；优先级 `P1`；权限 `CLIPBOARD_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: pasteboard set --raw --data <TEST_UTI=BASE64>；get --raw；设备测试app真实粘贴`
- 检查：public.png/public.jpeg已知图片逐方向/单表示与多表示；大数据用受控harness/stdin支持的真实入口避免argv限额，未成功不能借Viewer图像项替代
- 对照：`R-CLIPBOARD`。
- 附加前置：先用专属测试文本/图片替换个人剪贴板并记录可恢复基线；明确测试图且数据只留私密证据；确认当前CLI实际stdin能力


<a id="view"></a>
## VIEW Web Viewer、VNC与双向图像

前置：受控测试app/画面，浏览器和独立RFB3.8客户端；默认本地绑定，先记录剪贴板基线。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-VIEWER`，详见 UPSTREAM.md；覆盖：`viewer`。

### VIEW-001 默认PNG Web实时画面

- 适用：`MODERN`；优先级 `P0`；权限 `READ`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U viewer --local-unauthenticated web`
- 检查：127.0.0.1:8080显示实际手机连续画面；至少10次画面变化、帧尺寸和方向正确
- 对照：`R-VIEWER`。

### VIEW-002 HEVC持续解码

- 适用：`MODERN`；优先级 `P0`；权限 `MEDIA_INPUT`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U viewer --local-unauthenticated --hevc web`
- 检查：真实Display/RTP→外部FFmpeg→像素连续至少60秒；不是仅NAL/文件存在；记录PTS、实际帧率与FFmpeg版本
- 对照：`R-VIEWER`。
- 附加前置：受控测试app/画面，浏览器和独立RFB3.8客户端；默认本地绑定，先记录剪贴板基线；管理员+Wintun、FFmpeg在PATH

### VIEW-003 HEVC转向/参数变化

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: HEVC运行期间受控旋转、前后台切换`
- 检查：尺寸/关键帧变化后画面仍正确，输入坐标对应最新尺寸，不能沿用过期几何
- 对照：`R-VIEWER`。

### VIEW-004 Web输入与方向

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 本次Viewer页面操作测试app的tap/swipe/text/key/orientation`
- 检查：逐项真实效果及释放；没有真实效果不能凭HTTP200通过；不是CDP可信输入验收
- 对照：`R-VIEWER`。

### VIEW-005 独立VNC客户端

- 适用：`MODERN`；优先级 `P0`；权限 `MEDIA_INPUT`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U viewer --local-unauthenticated vnc`
- 检查：独立桌面RFB客户端连接127.0.0.1:5900；真实像素/方向/输入，不能拿服务自往返测试替代
- 对照：`R-VIEWER`。

### VIEW-006 VNC HEVC共享帧

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U viewer --local-unauthenticated --hevc vnc`
- 检查：独立客户端看到持续解码同源手机画面；重新连接不泄漏旧会话像素
- 对照：`R-VIEWER`。

### VIEW-007 Web/VNC双向文本

- 适用：`MODERN`；优先级 `P1`；权限 `CLIPBOARD_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: viewer --local-unauthenticated --share-clipboard web/vnc，两个模式分别测`
- 检查：设备→主机、主机→设备各中文/换行/emoji，实际粘贴；无无限重复或回环
- 对照：`R-VIEWER`。

### VIEW-008 双向PNG RGB/RGBA图像

- 适用：`MODERN`；优先级 `P0`；权限 `CLIPBOARD_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: VIEWER_IMAGES：Web与独立VNC逐方向粘贴专属RGB/半透明RGBA PNG`
- 检查：设备实际粘贴与画面/像素正确；RFB bit3为DIB v5、alpha不丢；不能以本地编码往返代替设备互通
- 对照：`R-VIEWER`。

### VIEW-009 双向JPEG图像

- 适用：`MODERN`；优先级 `P1`；权限 `CLIPBOARD_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: VIEWER_IMAGES：RGB/灰度JPEG逐方向传输；CMYK副本只做拒绝边界`
- 检查：实际图像内容、方向/尺寸正确；有损比较使用明确误差；CMYK/不支持格式准确拒绝而非损坏后成功
- 对照：`R-VIEWER`。

### VIEW-010 跨格式去重

- 适用：`MODERN`；优先级 `P1`；权限 `CLIPBOARD_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 相同传输像素的PNG/JPEG、副本以及真实变更连续粘贴`
- 检查：同像素不形成回环，真变更不被丢；JSON/base64脱敏摘要不当像素证据
- 对照：`R-VIEWER`。

### VIEW-011 独立桌面文本clipboard模式

- 适用：`MODERN`；优先级 `P1`；权限 `CLIPBOARD_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U viewer --local-unauthenticated clipboard`
- 检查：此独立模式仅文本：双向实际文本和退出恢复；不假设支持图像
- 对照：`R-VIEWER`。

### VIEW-012 慢客户端与断开重连

- 适用：`MODERN`；优先级 `P1`；权限 `MEDIA_INPUT`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 一端停消费/断连30秒，另一端继续读；随后恢复`
- 检查：仍有最新画面，资源不持续增长；记录进程工作集只是趋势非全heap证明；重连不重放HID/SET
- 对照：`R-VIEWER`。

### VIEW-013 媒体停止、清理失败与端口释放

- 适用：`MODERN`；优先级 `P0`；权限 `MEDIA_INPUT`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 用户Ctrl-C、拔线、本机decoder失效分开；记录前后Display状态及进程/监听端口`
- 检查：有限退出、原流停止；清理失败不自动重连；再次启动正常；不能用EOF冒充设备stop ACK
- 对照：`R-VIEWER`。

### VIEW-014 Viewer权限边界

- 适用：`MODERN`；优先级 `P1`；权限 `READ`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: AUTH_HTTP；Viewer错误Host/Origin/token/跨设备scope，VNC错误密码/owner token`
- 检查：拒绝者不能抓屏/注入输入/读剪贴板；有效权限仍可运行；默认localhost不能误暴露
- 对照：`R-AUTH`。


<a id="web"></a>
## WEB WebInspector、CDP固定客户端与WebDriver

前置：Safari Web Inspector已开；受控测试网页/真实page ID；launch/Selenium另需Remote Automation；固定Puppeteer25.12.0/Playwright1.63.0。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-WEB`，详见 UPSTREAM.md；覆盖：`webinspector`。

### WEB-001 真实标签页

- 适用：`BOTH`；优先级 `P0`；权限 `READ`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U webinspector opened-tabs --timeout 10`
- 检查：对应所选手机和真实页面；表达式=2、URL/标题实际匹配；shell有界退出
- 对照：`R-WEB`。

### WEB-002 原生求值

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U webinspector eval "1+1" --page-id <OBSERVED_PAGE_ID>`
- 检查：对应所选手机和真实页面；表达式=2、URL/标题实际匹配；shell有界退出
- 对照：`R-WEB`。

### WEB-003 launch受控URL

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U webinspector launch "$TestUrl"`
- 检查：对应所选手机和真实页面；表达式=2、URL/标题实际匹配；shell有界退出
- 对照：`R-WEB`。

### WEB-004 交互JS shell

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `45s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U webinspector js-shell "$TestUrl"`
- 检查：对应所选手机和真实页面；表达式=2、URL/标题实际匹配；shell有界退出
- 对照：`R-WEB`。

### WEB-005 CDP元数据与会话attach/detach

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U webinspector cdp --host 127.0.0.1 --port 9222 --local-unauthenticated`
- 检查：真实/json/version/list及WS attach→Runtime→detach；HTTP可达不是页面能力通过
- 对照：`R-WEB`。

### WEB-006 Runtime基本类型/异常/对象

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；evaluate字符串/数字/null/throw、getProperties、callFunctionOn、releaseObject；核对真实值/异常和对象寿命`
- 检查：evaluate字符串/数字/null/throw、getProperties、callFunctionOn、releaseObject；核对真实值/异常和对象寿命；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-007 页面正常realm生命周期

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；导航/reload/page关闭销毁旧context，旧ID求值拒绝；迟到frame-tree不复活旧realm`
- 检查：导航/reload/page关闭销毁旧context，旧ID求值拒绝；迟到frame-tree不复活旧realm；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-008 同源和跨源iframe

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；frame tree/安全origin/context对应正确；逐帧求值和导航，不能错误写main frame`
- 检查：frame tree/安全origin/context对应正确；逐帧求值和导航，不能错误写main frame；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-009 user/internal隔离world

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；同frame不同world独立值；context ID/世代/退役正确，normal索引仅用于导航`
- 检查：同frame不同world独立值；context ID/世代/退役正确，normal索引仅用于导航；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-010 Console对象上下文与时间

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；带原生对象injectedScriptId及实际WallTime的console准确关联；primitive-only缺证据不得伪造context=1/now`
- 检查：带原生对象injectedScriptId及实际WallTime的console准确关联；primitive-only缺证据不得伪造context=1/now；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-011 Page事件与导航参数

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；真实frame commit/加载事件顺序；不支持referrer/transition明确拒绝，不能悄悄丢字段`
- 检查：真实frame commit/加载事件顺序；不支持referrer/transition明确拒绝，不能悄悄丢字段；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-012 截图PNG和参数边界

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；真实PNG像素与手机相符；不支持的JPEG/clip等按当前契约明确拒绝，不能假成功`
- 检查：真实PNG像素与手机相符；不支持的JPEG/clip等按当前契约明确拒绝，不能假成功；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-013 Fetch请求继续与重写

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；受控页面fetch→pause→continue url/method/headers/body；测试服务器真实收到预期内容`
- 检查：受控页面fetch→pause→continue url/method/headers/body；测试服务器真实收到预期内容；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-014 Fetch fulfill/fail和响应阶段

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；受控请求分别fulfill/fail/Response-stage；页面实际结果、reason/base64与wire匹配`
- 检查：受控请求分别fulfill/fail/Response-stage；页面实际结果、reason/base64与wire匹配；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-015 Fetch disable/取消/rollback

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；暂停期间disable释放pending、移除精确stage/filter；失败原因与cleanup都保留，重启不残留旧filter`
- 检查：暂停期间disable释放pending、移除精确stage/filter；失败原因与cleanup都保留，重启不残留旧filter；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-016 Memory/Performance采样

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；enable→真实Memory.trackingUpdate→getMetrics→disable；设备elapsed秒/bytes正确，WebKit命名分类不冒充Chrome heap计数`
- 检查：enable→真实Memory.trackingUpdate→getMetrics→disable；设备elapsed秒/bytes正确，WebKit命名分类不冒充Chrome heap计数；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-017 GC与不支持指标

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；HeapProfiler.collectGarbage实际Heap.gc；无sample/无对应DOM计数明确错误，不返回零或常量`
- 检查：HeapProfiler.collectGarbage实际Heap.gc；无sample/无对应DOM计数明确错误，不返回零或常量；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-018 Puppeteer固定版本流程

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；本机真实Puppeteer25.12.0经过Rust桥操作真实手机；16个fixture门禁不能直接替代设备结果`
- 检查：本机真实Puppeteer25.12.0经过Rust桥操作真实手机；16个fixture门禁不能直接替代设备结果；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-019 Playwright默认初始化边界

- 适用：`BOTH`；优先级 `P1`；权限 `KNOWN_GAP`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；真实Playwright1.63.0默认connectOverCDP当前可能在Browser.setDownloadBehavior -32601被拒绝；记录KNOWN_GAP，不把负向门禁PASS当完整支持`
- 检查：真实Playwright1.63.0默认connectOverCDP当前可能在Browser.setDownloadBehavior -32601被拒绝；记录KNOWN_GAP，不把负向门禁PASS当完整支持；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-020 可信Input与下载OPEN

- 适用：`BOTH`；优先级 `P1`；权限 `KNOWN_GAP`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CDP_REAL_DEVICE；Input与Browser/Page下载save/cancel/events仍搁置；只检查明确Unsupported，不能JS dispatchEvent假作可信输入`
- 检查：Input与Browser/Page下载save/cancel/events仍搁置；只检查明确Unsupported，不能JS dispatchEvent假作可信输入；至少一个真实手机可观察断言，保留客户端轨迹和测试网页日志
- 对照：`R-WEB`。

### WEB-021 Selenium真实session

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U webinspector selenium --host 127.0.0.1 --port 4444 --local-unauthenticated`
- 检查：受控WebDriver客户端创建session、导航/标题/元素查询、删除session；实际页面可验证，不伪造通过
- 对照：`R-WEB`。

### WEB-022 WebDriver会话隔离/删除清理

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 同一手机两个明确page/session，删除其中一个，再Ctrl-C服务器`
- 检查：被删上下文不可再用，另一个正常；原WebInspector/forward有限退出；认证revocation后不能继续访问
- 对照：`R-WEB`。

### WEB-023 CDP/Driver访问控制与设备断开

- 适用：`BOTH`；优先级 `P1`；权限 `PAGE_WRITE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: AUTH_HTTP；错误token、撤销token、过期/错设备；再物理拔出目标`
- 检查：真实会话结束/拒绝并清理，不能只有metadata变化；未选中手机会话保持隔离
- 对照：`R-AUTH`。


<a id="xct"></a>
## XCT XCTest、WDA与统一工作流

前置：合法签名测试App/Runner已登记，DDI/开发服务可用；设计1通过、1故意失败、1慢测试和crash样例。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-XCTEST`，详见 UPSTREAM.md；覆盖：`runtest`, `runxctest`, `runwda`, `wda`。

### XCT-001 xctestrun实际完成

- 适用：`BOTH`；优先级 `P0`；权限 `TEST_APP_WRITE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U runtest "$Xctestrun" --wait --result-timeout-secs 120 --junit-output "$CaseDir/runtest.xml"`
- 检查：至少1真实case成功且有完成事件；JUnit/test count/terminal/退出语义一致；无--wait只算启动
- 对照：`R-XCTEST`。

### XCT-002 安装runner的bundle入口

- 适用：`BOTH`；优先级 `P0`；权限 `TEST_APP_WRITE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U runxctest --test-runner-bundle-id "$RunnerBundle" --xctest-config "$XctestConfig" --bundle-id "$TestBundle" --test <PASS_SELECTOR> --wait --timeout 120 --junit-output "$CaseDir/runxctest.xml"`
- 检查：真实测试方法、结果和JUnit一致，与runtest同一编排；go-ios两个命令名方向相反
- 对照：`R-XCTEST`。

### XCT-003 配置/target选择

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；多configuration/target选择与不存在选择明确错误`
- 检查：多configuration/target选择与不存在选择明确错误；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-004 故意失败与CI退出

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；1个真实失败case，JUnit failure和非通过退出，不能把已知测试失败当基础设施bug`
- 检查：1个真实失败case，JUnit failure和非通过退出，不能把已知测试失败当基础设施bug；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-005 部分结果和truncated输出

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；runner crash/异常结束后保留已完成case，缺terminal不能假通过`
- 检查：runner crash/异常结束后保留已完成case，缺terminal不能假通过；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-006 rerun不重放安装

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；只重跑选中失败测试，显式安装等已提交写不重复`
- 检查：只重跑选中失败测试，显式安装等已提交写不重复；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-007 安装→启动→日志→产物整链

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；获准测试IPA一次显式安装，执行/有限syslog/crash/JUnit/sidecar/cleanup全部有真实证据`
- 检查：获准测试IPA一次显式安装，执行/有限syslog/crash/JUnit/sidecar/cleanup全部有真实证据；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-008 启动未确认PID取消

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；启动中Ctrl-C或拔线；已知PID终止/服务先关；未知PID不猜杀，明确uncertain`
- 检查：启动中Ctrl-C或拔线；已知PID终止/服务先关；未知PID不猜杀，明确uncertain；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-009 有限取消与再次执行

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；安装/启动/结果收集中各取消；资源收尾后新合法测试可运行`
- 检查：安装/启动/结果收集中各取消；资源收尾后新合法测试可运行；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-010 两设备目标隔离

- 适用：`DUAL`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: XCTEST_REAL_DEVICE；同时连接D1和D2，显式目标运行；失败/stop不启动或杀错手机runner`
- 检查：同时连接D1和D2，显式目标运行；失败/stop不启动或杀错手机runner；每次保留选择、写操作次数、PID/日志/terminal/JUnit/cleanup
- 对照：`R-XCTEST`。

### XCT-011 启动WDA与forward关闭

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U runwda "$RunnerBundle" --once`
- 检查：WDA真实/status返回，runner与端口对应；stop后forward/进程收尾；不能只看监听端口
- 对照：`R-XCTEST`。

### XCT-012 WDA status

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 status`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-013 WDA 创建session

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 session --bundle-id "$TestBundle"`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-014 WDA source

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 source --session-id <OBSERVED_SESSION>`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-015 WDA 截图

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 screenshot "$CaseDir/wda.png"`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-016 WDA find

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 find <SID> <USING> <VALUE>`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-017 WDA click

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 click <SID> <ELEMENT>`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-018 WDA press-button

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 press-button <APPROVED_BUTTON> --session-id <SID>`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-019 WDA unlock

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 unlock`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-020 WDA send-keys

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 send-keys <SID> "ios-lab-test"`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。

### XCT-021 WDA swipe

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U wda --device-port 8100 swipe <SID> 100 300 100 100`
- 检查：仅在实际WDA会话和测试app内核对真实响应/屏幕效果；元素/session/PID不能用历史值
- 对照：`R-WDA`。


<a id="net"></a>
## NET 端口、通知、伴侣与网络profile

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-NETWORK`，详见 UPSTREAM.md；覆盖：`forward`, `dproxy`, `notify`, `companion`, `wifi`, `httpproxy`, `idam`。

### NET-001 USB端口forward

- 适用：`BOTH`；优先级 `P0`；权限 `SESSION_WRITE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U forward <FREE_HOST_PORT> <APPROVED_DEVICE_PORT> --once`
- 检查：测试app/WDA端口真实往返，非空监听而已；退出后端口释放，禁止转发未授权敏感服务
- 对照：`R-NETWORK`。

### NET-002 DTX服务proxy记录

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `90s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U dproxy service <APPROVED_SERVICE> --once --output "$CaseDir/dproxy"`
- 检查：真实受控客户端经proxy请求/回复；记录在仓库外private CaseDir，可解析但敏感payload不共享；清理后端口释放
- 对照：`R-NETWORK`。

### NET-003 通知 observe <SAFE_TEST_NOTIFICATION> --timeout 10

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U notify observe <SAFE_TEST_NOTIFICATION> --timeout 10`
- 检查：测试事件真实来源、名称、时间正确；没有事件时timeout准确而非伪造成功
- 对照：`R-NETWORK`。

### NET-004 通知 wait <SAFE_TEST_NOTIFICATION> --timeout 10

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U notify wait <SAFE_TEST_NOTIFICATION> --timeout 10`
- 检查：测试事件真实来源、名称、时间正确；没有事件时timeout准确而非伪造成功
- 对照：`R-NETWORK`。

### NET-005 通知 stream <SAFE_TEST_NOTIFICATION> --timeout 10

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U notify stream <SAFE_TEST_NOTIFICATION> --timeout 10`
- 检查：测试事件真实来源、名称、时间正确；没有事件时timeout准确而非伪造成功
- 对照：`R-NETWORK`。

### NET-006 通知 springboard --timeout 10

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U notify springboard --timeout 10`
- 检查：测试事件真实来源、名称、时间正确；没有事件时timeout准确而非伪造成功
- 对照：`R-NETWORK`。

### NET-007 安全通知post/roundtrip

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: notify post <SAFE_TEST_NOTIFICATION>；notify roundtrip <SAFE_TEST_NOTIFICATION> --timeout 10`
- 检查：仅com.example.ios-lab.<run_id>之类批准无副作用通知；真实observer收到，不发布Apple状态通知
- 对照：`R-NETWORK`。

### NET-008 Companion list

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U companion list`
- 检查：有实际配对手表才测；没有伴侣可测空列表，get/listen实机场景BLOCKED
- 对照：`R-NETWORK`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；真实配对伴侣及操作许可

### NET-009 Companion get <OBSERVED_COMPANION_UDID> <APPROVED_KEY>

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U companion get <OBSERVED_COMPANION_UDID> <APPROVED_KEY>`
- 检查：有实际配对手表才测；没有伴侣可测空列表，get/listen实机场景BLOCKED
- 对照：`R-NETWORK`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；真实配对伴侣及操作许可

### NET-010 Companion listen --timeout 10

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U companion listen --timeout 10`
- 检查：有实际配对手表才测；没有伴侣可测空列表，get/listen实机场景BLOCKED
- 对照：`R-NETWORK`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；真实配对伴侣及操作许可

### NET-011 Companion forward/stop

- 适用：`BOTH`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `30s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: companion forward <APPROVED_REMOTE_PORT> --timeout 10；companion stop <SAME_PORT>`
- 检查：返回的是设备侧CompanionProxyServicePort；连接并stop同端口，不能当host监听
- 对照：`R-NETWORK`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；真实伴侣和测试服务

### NET-012 IDAM读取

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U idam get`
- 检查：真实当前配置；硬件/服务不支持与上游比对
- 对照：`R-NETWORK`。

### NET-013 IDAM切换/恢复

- 适用：`BOTH`；优先级 `P1`；权限 `LAB_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: idam set on；idam set off，最后恢复原值`
- 检查：人工确认接口/音频路由影响后实际观察并恢复；不可让网络断开影响其它用例
- 对照：`R-NETWORK`。

### NET-014 测试Wi-Fi配置安装/删除

- 适用：`BOTH`；优先级 `P1`；权限 `NETWORK_PROFILE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 在受控子进程环境设置IOS_WIFI_PASSWORD，wifi install <LAB_SSID>（不传--password）；wifi remove <LAB_SSID> --force；结束清理环境变量`
- 检查：仅批准测试SSID；真实连通性和删除后的原网络恢复；密码不出现在argv/报告/历史
- 对照：`R-NETWORK`。

### NET-015 监督HTTP proxy设置/移除

- 适用：`BOTH`；优先级 `P1`；权限 `NETWORK_PROFILE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: httpproxy set <LAB_PROXY_HOST> <PORT> --p12 <SUPERVISOR> --force；httpproxy remove`
- 检查：受监督测试机/测试代理可用才运行；真实HTTP经过代理，最后原配置还原
- 对照：`R-NETWORK`。


<a id="profile"></a>
## PROFILE 配置、Provisioning与受监督功能

前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-PROFILES`，详见 UPSTREAM.md；覆盖：`profiles`, `provisioning`, `mdm`, `prepare`, `preboard`, `arbitration`。

### PROFILE-001 profiles list

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U profiles list`
- 检查：仅对应目标的实际profile/监督状态；导出或权限不足不混同服务成功
- 对照：`R-PROFILES`。

### PROFILE-002 profiles show <TEST_PROFILE_QUERY>

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U profiles show <TEST_PROFILE_QUERY>`
- 检查：仅对应目标的实际profile/监督状态；导出或权限不足不混同服务成功
- 对照：`R-PROFILES`。

### PROFILE-003 profiles cloud-config

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U profiles cloud-config`
- 检查：仅对应目标的实际profile/监督状态；导出或权限不足不混同服务成功
- 对照：`R-PROFILES`。

### PROFILE-004 profiles stored

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U profiles stored`
- 检查：仅对应目标的实际profile/监督状态；导出或权限不足不混同服务成功
- 对照：`R-PROFILES`。

### PROFILE-005 provisioning list

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U provisioning list`
- 检查：仅对应目标的实际profile/监督状态；导出或权限不足不混同服务成功
- 对照：`R-PROFILES`。

### PROFILE-006 provisioning show <TEST_PROFILE_UUID>

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U provisioning show <TEST_PROFILE_UUID>`
- 检查：仅对应目标的实际profile/监督状态；导出或权限不足不混同服务成功
- 对照：`R-PROFILES`。

### PROFILE-007 配置profile安装/删除

- 适用：`BOTH`；优先级 `P1`；权限 `PROFILE_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: profiles add "$TestMobileconfig"；profiles remove <EXACT_TEST_IDENTIFIER>`
- 检查：只用无VPN/MDM接管副作用的批准测试配置；设备实际显示，删除后基线恢复
- 对照：`R-PROFILES`。

### PROFILE-008 Provisioning安装/移除

- 适用：`BOTH`；优先级 `P1`；权限 `PROFILE_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: provisioning install <TEST_MOBILEPROVISION>；provisioning remove <EXACT_TEST_UUID>`
- 检查：签名合法且专属测试profile；关联测试app可启动，删除只限本次profile
- 对照：`R-PROFILES`。

### PROFILE-009 Provisioning私密导出

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: provisioning export <TEST_UUID> <NEW_OUTPUT>；provisioning dump <NEW_DIR>`
- 检查：导出格式/UUID与真实profile一致；内容只存private且绝不覆盖其它证书
- 对照：`R-PROFILES`。

### PROFILE-010 mdm security-info --p12 "$SupervisorP12"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U mdm security-info --p12 "$SupervisorP12"`
- 检查：需真实监督设备和合法证书；只记录状态/hash不暴露证书或unlock token
- 对照：`R-PROFILES`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；受监督设备及用户证书；通常普通两机BLOCKED

### PROFILE-011 mdm passcode-present --p12 "$SupervisorP12"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U mdm passcode-present --p12 "$SupervisorP12"`
- 检查：需真实监督设备和合法证书；只记录状态/hash不暴露证书或unlock token
- 对照：`R-PROFILES`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；受监督设备及用户证书；通常普通两机BLOCKED

### PROFILE-012 mdm fetch-unlock-token --p12 "$SupervisorP12" --output "$PrivateRoot/unlock-token"

- 适用：`BOTH`；优先级 `P1`；权限 `SENSITIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U mdm fetch-unlock-token --p12 "$SupervisorP12" --output "$PrivateRoot/unlock-token"`
- 检查：需真实监督设备和合法证书；只记录状态/hash不暴露证书或unlock token
- 对照：`R-PROFILES`。
- 附加前置：已明确选中设备、已有信任；记录锁定/开发者模式/DDI状态；受监督设备及用户证书；通常普通两机BLOCKED

### PROFILE-013 mdm clear-passcode --p12 "$SupervisorP12" --token <PRIVATE_TOKEN_FILE> --force

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U mdm clear-passcode --p12 "$SupervisorP12" --token <PRIVATE_TOKEN_FILE> --force`
- 检查：逐项人工批准后才操作专属设备；监督/密码/恢复状态变化不属于默认测试写入；无许可NOT_RUN
- 对照：`R-PROFILES`。

### PROFILE-014 mdm clear-screen-time-password --p12 "$SupervisorP12" --force

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U mdm clear-screen-time-password --p12 "$SupervisorP12" --force`
- 检查：逐项人工批准后才操作专属设备；监督/密码/恢复状态变化不属于默认测试写入；无许可NOT_RUN
- 对照：`R-PROFILES`。

### PROFILE-015 prepare

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U prepare`
- 检查：逐项人工批准后才操作专属设备；监督/密码/恢复状态变化不属于默认测试写入；无许可NOT_RUN
- 对照：`R-PROFILES`。

### PROFILE-016 preboard create

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U preboard create`
- 检查：逐项人工批准后才操作专属设备；监督/密码/恢复状态变化不属于默认测试写入；无许可NOT_RUN
- 对照：`R-PROFILES`。

### PROFILE-017 preboard commit

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U preboard commit`
- 检查：逐项人工批准后才操作专属设备；监督/密码/恢复状态变化不属于默认测试写入；无许可NOT_RUN
- 对照：`R-PROFILES`。

### PROFILE-018 本地监督测试证书

- 适用：`HOST`；优先级 `P1`；权限 `LOCAL_SECRET`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U prepare create-cert "$PrivateRoot/lab-supervisor"`
- 检查：仅创建本地测试证书，文件私密保存；不能拿它冒充现有设备真实监督身份
- 对照：`R-PROFILES`。

### PROFILE-019 Arbitration version

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U arbitration version`
- 检查：真实协议版本/会话登记与退出；check-in/out单独记录且成对收尾
- 对照：`R-PROFILES`。

### PROFILE-020 Arbitration check-in

- 适用：`BOTH`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U arbitration check-in`
- 检查：真实协议版本/会话登记与退出；check-in/out单独记录且成对收尾
- 对照：`R-PROFILES`。

### PROFILE-021 Arbitration check-out

- 适用：`BOTH`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U arbitration check-out`
- 检查：真实协议版本/会话登记与退出；check-in/out单独记录且成对收尾
- 对照：`R-PROFILES`。


<a id="act"></a>
## ACT 激活、恢复与明确搁置范围

前置：普通设备正常模式；恢复/擦除/激活等均不由默认读测试授权。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-RESTORE`，详见 UPSTREAM.md；覆盖：`activation`, `amfi`, `restore`, `erase`。

### ACT-001 激活 state

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U activation state`
- 检查：只读激活状态/脱敏session摘要；不把nonce/session值放共享证据
- 对照：`R-RESTORE`。

### ACT-002 激活 session-info

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U activation session-info`
- 检查：只读激活状态/脱敏session摘要；不把nonce/session值放共享证据
- 对照：`R-RESTORE`。

### ACT-003 激活 info

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U activation info`
- 检查：只读激活状态/脱敏session摘要；不把nonce/session值放共享证据
- 对照：`R-RESTORE`。

### ACT-004 激活变更 activate

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U activation activate`
- 检查：独立授权Apple联网/激活状态变更；无此授权NOT_RUN，不自动重试nonce写入
- 对照：`R-RESTORE`。

### ACT-005 激活变更 activate --now

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U activation activate --now`
- 检查：独立授权Apple联网/激活状态变更；无此授权NOT_RUN，不自动重试nonce写入
- 对照：`R-RESTORE`。

### ACT-006 激活变更 itunes-activate

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U activation itunes-activate`
- 检查：独立授权Apple联网/激活状态变更；无此授权NOT_RUN，不自动重试nonce写入
- 对照：`R-RESTORE`。

### ACT-007 激活变更 deactivate --force

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `300s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U activation deactivate --force`
- 检查：独立授权Apple联网/激活状态变更；无此授权NOT_RUN，不自动重试nonce写入
- 对照：`R-RESTORE`。

### ACT-008 Developer Mode reveal-developer-mode

- 适用：`BOTH`；优先级 `P1`；权限 `REBOOT`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U amfi reveal-developer-mode`
- 检查：人工操作手机重启/确认后读回实际状态；先记录原状态，无权限不强行绕过
- 对照：`R-RESTORE`。

### ACT-009 Developer Mode enable-developer-mode

- 适用：`BOTH`；优先级 `P1`；权限 `REBOOT`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U amfi enable-developer-mode`
- 检查：人工操作手机重启/确认后读回实际状态；先记录原状态，无权限不强行绕过
- 对照：`R-RESTORE`。

### ACT-010 恢复服务 preflight-info

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore preflight-info`
- 检查：仅在实际所需已授权模式和服务存在时读取；正常模式没有restored不当缺陷，不能为测试自动进恢复
- 对照：`R-RESTORE`。
- 附加前置：普通设备正常模式；恢复/擦除/激活等均不由默认读测试授权；已有模式/服务前置；否则BLOCKED

### ACT-011 恢复服务 nonces

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore nonces`
- 检查：仅在实际所需已授权模式和服务存在时读取；正常模式没有restored不当缺陷，不能为测试自动进恢复
- 对照：`R-RESTORE`。
- 附加前置：普通设备正常模式；恢复/擦除/激活等均不由默认读测试授权；已有模式/服务前置；否则BLOCKED

### ACT-012 恢复服务 app-parameters

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore app-parameters`
- 检查：仅在实际所需已授权模式和服务存在时读取；正常模式没有restored不当缺陷，不能为测试自动进恢复
- 对照：`R-RESTORE`。
- 附加前置：普通设备正常模式；恢复/擦除/激活等均不由默认读测试授权；已有模式/服务前置；否则BLOCKED

### ACT-013 恢复服务 events --timeout-secs 10

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `25s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore events --timeout-secs 10`
- 检查：仅在实际所需已授权模式和服务存在时读取；正常模式没有restored不当缺陷，不能为测试自动进恢复
- 对照：`R-RESTORE`。
- 附加前置：普通设备正常模式；恢复/擦除/激活等均不由默认读测试授权；已有模式/服务前置；否则BLOCKED

### ACT-014 IPSW只读预检

- 适用：`BOTH`；优先级 `P1`；权限 `LOCAL_FIXTURE`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore preflight-requests --ipsw "$ApprovedIpsw"`
- 检查：真实机型/board/chip/build/identity与离线IPSW匹配；检查payload不等于签名/可刷写
- 对照：`R-RESTORE`。
- 附加前置：普通设备正常模式；恢复/擦除/激活等均不由默认读测试授权；真实目标匹配IPSW，没有文件BLOCKED

### ACT-015 Update dry-run边界

- 适用：`BOTH`；优先级 `P1`；权限 `KNOWN_GAP`；外层时间上限参考 `120s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore update --ipsw "$ApprovedIpsw" --dry-run`
- 检查：只检视支持的预检行为；F06完整Update未完成，不能记真实恢复PASS
- 对照：`R-RESTORE`。

### ACT-016 恢复模式控制 enter-recovery

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore enter-recovery`
- 检查：当前搁置范围不自动执行；如用户后续明确批准则单独记录模式/重识别/退出，不把它当完整Update
- 对照：`R-RESTORE`。

### ACT-017 恢复模式控制 delay-recovery-image

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore delay-recovery-image`
- 检查：当前搁置范围不自动执行；如用户后续明确批准则单独记录模式/重识别/退出，不把它当完整Update
- 对照：`R-RESTORE`。

### ACT-018 恢复模式控制 restore-lang <APPROVED_LANG>

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore restore-lang <APPROVED_LANG>`
- 检查：当前搁置范围不自动执行；如用户后续明确批准则单独记录模式/重识别/退出，不把它当完整Update
- 对照：`R-RESTORE`。

### ACT-019 恢复模式控制 reboot --force

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U restore reboot --force`
- 检查：当前搁置范围不自动执行；如用户后续明确批准则单独记录模式/重识别/退出，不把它当完整Update
- 对照：`R-RESTORE`。

### ACT-020 整机擦除

- 适用：`BOTH`；优先级 `P1`；权限 `DESTRUCTIVE`；外层时间上限参考 `1800s`（不是产品默认承诺）。
- 步骤/入口：`& $RustIos --udid $U erase --force`
- 检查：必须用户对专属设备单独授权并已备份；默认NOT_RUN；无执行证据不通过
- 对照：`R-RESTORE`。

### ACT-021 完整Update搁置项

- 适用：`BOTH`；优先级 `P1`；权限 `KNOWN_GAP`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 仅源码/--help核对；禁止向真实手机运行update --force或真实TSS`
- 检查：F06缺模式/nonce/TSS/IMG4/FDR等完整执行器；记录KNOWN_GAP，不能写真实恢复已验收
- 对照：`R-RESTORE`。


<a id="sdk"></a>
## SDK Rust、Python及C公共接口

前置：绑定/示例来自当前源码和同一测试基线；已有信任，配对字节走私密输入不进日志。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-SDK`，详见 UPSTREAM.md；覆盖：。

### SDK-001 Rust发现/连接/服务/close

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE Rust；使用当前ios-core公共入口的显式UDID小型已审核harness`
- 检查：list_devices/watch_devices/connect、选定服务只读操作、drop/close真实执行；CLI通过不等于Rust SDK单独调用通过
- 对照：`R-SDK`。

### SDK-002 公开统一Rust workflow

- 适用：`BOTH`；优先级 `P1`；权限 `TEST_APP_WRITE`；外层时间上限参考 `240s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE Rust；外部程序调用 ios_cli::TestWorkflowEngine::run`
- 检查：实际测试case/JUnit/cleanup经公共入口完成，不能只构造plan或用workflow_api假服务测试代替设备
- 对照：`R-SDK`。

### SDK-003 Python模块来源/设备列表

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE Python；ios_rs.__file__/list_devices`
- 检查：module来自当前构建/本仓库；两机返回正确；不误装PyPI旧包
- 对照：`R-SDK`。

### SDK-004 Python Device同步查询/服务

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE Python；with ios_rs.open_device($U)；product_version/list_apps/list_files/read_file/read_syslog`
- 检查：实际字段/应用/测试文件hash/测试日志正确；每种方法独立subcheck、with退出释放
- 对照：`R-SDK`。

### SDK-005 Python Device异步查询/服务

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE Python；核对当前async_*方法后逐个await`
- 检查：与同步接口同语义，取消后原服务不污染新请求；GIL不长期阻塞事件循环
- 对照：`R-SDK`。

### SDK-006 Python有限DeviceSession

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE Python；DeviceSession(udid, pair_record_bytes, endpoint)；query/query_async/service/service_async；service操作值apps/files/read_file/logs`
- 检查：读字段/应用/AFC/日志分别真实；deadline/cancel/错误分类及close实际生效；不以Device类替代Session
- 对照：`R-SDK`。

### SDK-007 Python tunnel及asyncio桥

- 适用：`MODERN`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: ios_rs.start_tunnel($U, mode="userspace")；connect_info/services/asyncio_proxy；finally close`
- 检查：真实RSD端口可达、service_ports/features正确，context退出恢复asyncio；repeat close稳定
- 对照：`R-SDK`。

### SDK-008 Python+pmd本地桥互通

- 适用：`MODERN`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: crates/ios-py/examples/pymobiledevice3_coredevice_bridge.py --udid <UDID>`
- 检查：同环境本地pmd通过Rust隧道访问真实RSD；metadata/可选--probe-coredevice分开，不当完整diagnostics
- 对照：`R-SDK`。

### SDK-009 C ABI来源/设备查询

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE C；list_devices.c/device_info.c，当前DLL和header`
- 检查：版本/基本字段来自真实目标；status/last_error_class/owned string读取和free匹配，测试harness可复现
- 对照：`R-SDK`。

### SDK-010 C应用/AFC/syslog服务

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE C；device_services.c按fixture调整读取路径`
- 检查：list_apps/list_files/read_file/syslog逐项，文件必须是本次真实fixture，不能默认/iTunesArtwork存在；hash和close正确
- 对照：`R-SDK`。

### SDK-011 C DeviceSession有限/取消/释放

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE C；公开header会话query/service、cancel/close/checked free`
- 检查：同线程读取原生error class，deadline/取消与再次查询有明确语义；绝不测试use-after-free指针
- 对照：`R-SDK`。

### SDK-012 C真实隧道/RSD metadata

- 适用：`MODERN`；优先级 `P1`；权限 `SESSION_WRITE`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: SDK_REAL_DEVICE C；显式目标tunnel handle/RSD JSON/端口/close`
- 检查：真实服务名/port/features与同机RSD一致，关闭后连接释放；不能仅返回缓存port判连通
- 对照：`R-SDK`。

### SDK-013 直接Core服务接口补充

- 适用：`BOTH`；优先级 `P1`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: COVERAGE.md逐service映射；如声称SDK完整覆盖须按公共client在真实服务上执行同功能`
- 检查：CLI所用service路径覆盖与SDK直接调用分开；未写harness/未执行记BLOCKED，不用方法名存在作真机PASS
- 对照：`R-SDK`。


<a id="cross"></a>
## CROSS 两机、状态切换和完成复核

前置：D1/D2已登记；两个目标身份始终显式；先完成各自基础用例。
清理：关闭本用例创建的服务/进程；恢复用例修改前状态。
对照索引：`R-GENERAL`，详见 UPSTREAM.md；覆盖：。

### CROSS-001 双机查询和停止隔离

- 适用：`DUAL`；优先级 `P0`；权限 `READ`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: D1 syslog/文件读等待期间D2 info/截图；再交换`
- 检查：B查询/停止响应时间记录，与预定1秒调度目标区分总设备耗时；不能A阻塞时错误退出B
- 对照：`R-GENERAL`。

### CROSS-002 单机拔出不影响另一机

- 适用：`DUAL`；优先级 `P0`；权限 `READ`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: D1持续只读时人工拔D2；反向再测`
- 检查：目标断连报告准确，另一台可继续，重新插回身份不串台；不杀共享usbmux服务
- 对照：`R-GENERAL`。

### CROSS-003 锁定/解锁及开发条件变化

- 适用：`DUAL`；优先级 `P1`；权限 `READ`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 两机分别锁屏/解锁，重复info/screenshot/开发服务查询`
- 检查：只对受影响服务分类限制；解锁后正常恢复；若需改开发者模式先授权重启
- 对照：`R-GENERAL`。

### CROSS-004 USB与Wi-Fi路径切换

- 适用：`DUAL`；优先级 `P1`；权限 `READ`；外层时间上限参考 `180s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: 各机USB成功→拔USB留Wi-Fi→网络中断→恢复`
- 检查：每次记录真实transport/endpoint而非只看UDID；两条路线结果独立，不将缓存metadata当连通
- 对照：`R-GENERAL`。

### CROSS-005 完成后还原与遗留检查

- 适用：`DUAL`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: END_OF_RUN：设置、定位、剪贴板、测试app/目录/profile、隧道/路由/子进程/端口逐项核对`
- 检查：所有授权状态回到基线或明确列残留/无法恢复；任何cleanup失败有issue，不用上游同现象掩盖新增残留
- 对照：`R-GENERAL`。

### CROSS-006 上游差分复核与问题包

- 适用：`DUAL`；优先级 `P0`；权限 `READ`；外层时间上限参考 `60s`（不是产品默认承诺）。
- 步骤/入口：`RECIPE: CLASSIFICATION.md；将候选差异按Rust→reference→Rust单独复现`
- 检查：同条件同现象标UPSTREAM_MATCH；无等价/环境未齐不判项目FAIL；最小命令/全部证据可重现
- 对照：`R-GENERAL`。
