# raw-projects上游对照手册

用户要求：候选失败必须检查本地 `raw-projects/go-ios` 和 `raw-projects/pymobiledevice3` 的相同行为。**证实同条件同现象时最终记 UPSTREAM_MATCH，不记项目FAIL，也不记PASS。**判定细则见 [CLASSIFICATION.md](CLASSIFICATION.md)。

本地源码是本轮对照依据，不需要联网搜索最新教程。记录实际checkout HEAD/dirty、构建/venv来源、实际工具版本和二进制hash；不pull、不checkout别的revision、不丢弃已有 `uv.lock` 或其它现场。若上游自身在Windows/该iOS route上不支持，记录其不适用，不能用错误路由的人为失败豁免Rust。

## 工具命名与语法

```powershell
$Repo = 'C:\Users\wangbaofeng\RustroverProjects\rust-ios-device'
$RustIos = Join-Path $Repo 'target\debug\ios.exe'
$GoIos = '<本轮构建或来源已核对的go-ios.exe绝对路径>'
$PmdPython = '<raw-projects/pymobiledevice3对应venv的python.exe绝对路径>'
# $U必须是本次真实选定手机，完整值只留private证据。
& $GoIos --help
& $PmdPython -m pymobiledevice3 --help
```

不要在PATH上混用两个都叫ios的可执行文件。Rust全局 `--udid` 放子命令前；go-ios用其docopt语法，pmd很多 `--udid` 在叶子命令层，还可能要求service-provider/`--rsd`路线。每个模板执行前可查 `--help` 和本地源码确认。参数拼写/位置不对的usage错误不是同等功能失败。

本轮静态核对的版本：go-ios `ca2c56d2c294ff702cadf4821edd30b069d3a264`；pmd `03ba2270c0c80df77c37f65adc1967595970d31f`。源码路径下表相对各自raw-projects根目录。` <...>`为需现场替换的值，不可复制后原样执行。

## 对照索引

| 索引 | 功能及可用入口 | 比较什么 / 不能替代什么 | 本地依据 |
| --- | --- | --- | --- |
| R-GENERAL | 下列最接近的服务索引；先核对help | 无等价写NO_COMPARATOR，不硬凑相似命令 | 两上游README/CLI定义 |
| R-NONE | 主机路径/构建/本项目参数等没有一一对应 | 仍可验证本项目自身契约，但不能伪造上游结果 | 当前Rust源码/帮助 |
| R-DISCOVERY | go：`list` / `listen`；pmd：`usbmux list`、`bonjour rsd`等实际bonjour子命令 | 对应设备、transport、广告/事件；bonjour发现不证明信任/可连服务 | go `main.go`；pmd `cli/usbmux.py`、`cli/bonjour.py` |
| R-INFO | go：`info --udid=<U>`、`batterycheck`、`batteryregistry`、`diskspace`、`mobilegestalt`；pmd：`lockdown info --udid <U>`及diagnostics同字段 | 字段值/类型/单位；pmd顶层info是Lockdown信息而不是Rust所有CoreDevice字段 | go `main.go`；pmd `cli/lockdown.py`、`cli/diagnostics.py` |
| R-PAIR | go现有pair/readpair；pmd lockdown配对/查询 | 仅在有范围授权时新增pair；原信任摘要不能等同RemotePairing完成 | go `main.go`、`cmd_tunnel.go`；pmd lockdown/remote CLI |
| R-DDI | go：`image list/mount/auto`按help；pmd：`mounter auto-mount`及`cryptex`实际子命令 | 镜像/identity/设备可用性；不能仅看相同文件存在 | go image实现；pmd `cli/mounter.py`、`cli/cryptex.py`、`services/mobile_image_mounter.py` |
| R-TUNNEL | go：`tunnel start --userspace --udid=<U>`并使用明确既有pair-record路径；pmd：`remote`实际隧道/RSD入口 | 同真实iOS小版本、同route和权限；pmd用户态路线不能无条件替代17.0–17.3特权路线 | go `cmd_tunnel.go`、`ios/tunnel/`；pmd `cli/remote.py`、`remote/`、`AGENTS.md` |
| R-AUTH | 上游manager若有相同认证/设备scope用其明确入口，否则NO_COMPARATOR | 本项目token/Host/Origin/撤销保证不能用无认证上游成功来否定或豁免 | Rust `access.rs`、`docs/management-access.md` |
| R-APPS | go：`apps`、`install --path=<IPA>`、launch/kill；pmd：`apps list/install/uninstall`及`developer dvt/core-device`同等操作 | 同bundle/version/hash与设备真实效果；InstallationProxy与CoreDevice route需标明 | go `main.go`；pmd `cli/apps.py`、`cli/developer/` |
| R-FILE | go：`fsync`帮助确定ls/pull/push等；pmd：`afc`实际子命令或交互shell | 同AFC/HouseArrest域、远程fixture/hash、权限；CoreDevice fileservice不直接拿AFC替代 | go `main.go`和AFC实现；pmd `cli/afc.py`及file service实现 |
| R-BACKUP | pmd：`backup2`命令组，按当前help确认backup/restore等；go有等价入口才使用 | 同备份协议/密码/fixture；host本地提取与device-side unback/extract不是同一个动作 | pmd `cli/backup.py`（命令名backup2）、`services/mobilebackup2.py`；go当前help |
| R-LOGS | go：syslog/pcap/crash/trace等；pmd：`syslog live`、pcap、crash、btlogger实际子命令 | 同一时间窗口的测试标记、同报告、同格式与真实包；流退出0不代表收到事件 | go `main.go`；pmd `cli/syslog.py`、`cli/pcap.py`、`cli/crash.py`、`cli/btlogger.py` |
| R-DEVELOPER | go：`ps/sysmontap/launch/kill/...`；pmd：`developer dvt`和diagnostics同等命令 | DTX真实服务、采样单位/PID、真实输入或设备效果；未公开指标不补0 | go `ios/instruments/`；pmd `cli/developer/dvt/`、`cli/diagnostics.py` |
| R-SCREEN | go截图；pmd传统developer screenshot/现代screen-capture，见下例 | 同一受控画面/方向/尺寸；截图不是视频、HID、VNC或图像剪贴板等价 | go截图实现；pmd `cli/developer/__init__.py`、`cli/developer/core_device.py` |
| R-COREDEVICE | pmd：`developer core-device` configuration/rotate/display/HID实际命令 | 对应具体原生feature，服务/依赖和状态一致；无同等stop观察时记NO_COMPARATOR | pmd `cli/developer/core_device.py`、`remote/core_device/` |
| R-CLIPBOARD | go：`pasteboard get/set`；pmd：`developer core-device paste`、`copy`与monitor/实际剪贴板操作；定位对照developer对应location | paste=从设备读、copy=写设备，按help确认；resolve/export不一定有上游同等接口 | go main/pasteboard；pmd core-device和location CLI |
| R-VIEWER | pmd display/剪贴板可比较对应子功能；go screenshot仅比较截图 | 无完整等价VNC/Web Viewer就NO_COMPARATOR；不可用截图成功直接判输入/HEVC/图像剪贴板 | pmd core-device Display/pasteboard；Rust `docs/viewer.md` |
| R-WEB | go `webinspector list/eval/cdp`；pmd `webinspector opened-tabs/cdp/js-shell`，见下例 | 同手机/page ID/表达式/固定客户端命令及真实页面；两工具内target ID可不同，要按页面身份对应 | go webinspector入口；pmd `cli/webinspector.py` |
| R-XCTEST | go两个命令名与Rust相反，见下例；pmd XCTest/developer入口按help选同场景 | 同runner/xctestrun/test selector、至少1 case、终态/JUnit/cleanup；exit0和有XML均不够 | go `main.go`、`cmd_device_testing.go`；pmd developer testmanager实现 |
| R-WDA | 连接同一真实WDA的标准HTTP API；上游启动runner只作启动对照 | HTTP session/element真实效果；不同WDA版本/签名不直接比较 | go WDA/developer；当前Rust `cmd/wda.rs` |
| R-NETWORK | go：forward/notifications/companion等同等命令；pmd：`usbmux forward`、notification/companion、profile相关入口 | 真正TCP往返、通知名/来源、伴侣端口；companion forward不是host forward | 两上游main/对应CLI和service模块 |
| R-PROFILES | go：profile/provision/prepare等；pmd：profile/provision/mobile_config/preboard/arbitration实际入口 | 同监督状态、证书、具体identifier，所有写入范围相同；不因缺监督重置手机 | 两上游CLI和MCInstall/Misagent实现 |
| R-RESTORE | pmd restore/activation等实际读取/预检入口；go有明确等价才用 | F06完整Update仍搁置，禁止为对照启动真实刷机/TSS；dry-run不代表设备恢复 | pmd `cli/restore.py`、`restore/`及activation CLI；Rust restore声明 |
| R-SDK | 同设备/服务字段用上述工具；C/Python/Rust语言合同没有逐个上游等价 | 语义对照与ABI/GIL/内存/取消所有权分开；NO_COMPARATOR不自动FAIL | 当前SDK/header及上游service |

表中“按help”的条目是功能映射，不是可直接复制的argv。若没有精确等价命令，查本地源码或可用交互API，保留所用脚本；找不到就NO_COMPARATOR，禁止替换成不相关查询。以下给出已核对的常用具体模板。

## 已核对的常用模板

```powershell
# 基本信息：注意两个ios不能靠PATH区分
& $GoIos info --udid="$U"
& $PmdPython -m pymobiledevice3 lockdown info --udid "$U"

# WebInspector：各工具必须重新取其本次page ID并对应同一个测试页面
& $GoIos webinspector list --udid="$U"
& $GoIos webinspector eval <GO_PAGE_ID> '1+1' --udid="$U"
& $PmdPython -m pymobiledevice3 webinspector opened-tabs --udid "$U"
& $PmdPython -m pymobiledevice3 webinspector js-shell --udid "$U" --bundle-id com.apple.mobilesafari
# 在交互shell执行相同表达式，随后正常退出。

# CDP server分别独立启动/停止；不能与Rust占同端口
& $GoIos webinspector cdp --host=127.0.0.1 --port=9222 --udid="$U"
& $PmdPython -m pymobiledevice3 webinspector cdp --udid "$U" --host 127.0.0.1 --port 9222

# 截图：go与pmd输出分别使用新文件
& $GoIos screenshot --udid="$U" --output="$CaseDir\go.png"
& $PmdPython -m pymobiledevice3 developer screenshot "$CaseDir\pmd.png" --udid "$U"
# >=17且实际CoreDevice路线：
& $PmdPython -m pymobiledevice3 developer core-device screen-capture screenshot "$CaseDir\pmd-core.png" --udid "$U"
```

截图路线以实际系统支持为准，不用上面两个pmd命令强求同一设备都可用。WebDriver的pmd `shell` 是交互式Python API，不是Rust `selenium` HTTP服务器完整等价物。

### XCTest容易用反

| 本项目 | go-ios本地对照 |
| --- | --- |
| `runtest <FILE.xctestrun>` | **`runxctest --xctestrun-file-path=<FILE>`** |
| `runxctest --test-runner-bundle-id ... --xctest-config ...` | **`runtest --test-runner-bundle-id=... --xctest-config=...`** |

```powershell
& $GoIos runxctest --udid="$U" --xctestrun-file-path="$Xctestrun" --junit-output="$CaseDir\go-xctestrun.xml"
& $GoIos runtest --udid="$U" --test-runner-bundle-id="$RunnerBundle" --xctest-config="$XctestConfig" --bundle-id="$TestBundle" --test-to-run=<SAME_SELECTOR> --junit-output="$CaseDir\go-bundle.xml"
```

本地go-ios `cmd_device_testing.go` 的部分运行错误仅写日志，进程仍可能返回0或生成XML。因此必须查看真实测试case数、failure、terminal与日志；不得用上游exit0制造“Rust不如上游”假差异，也不能用空XML当成功。

### 隧道和工作目录

go-ios可按本地帮助使用 `tunnel start --userspace --udid=<U> --pair-record-path=<明确已有凭据目录>`。其默认工作目录可能写 `selfIdentity.plist` 等身份材料，不能在共享证据目录运行或把它当纯只读命令。优先在private目录运行且显式指定既有身份路径，记录其副作用。

pmd当前非macOS的17.0–17.3路线存在Userspace限制，可能需要具权限的tunneld；Windows的 `--native` / `--daemonize` 不能按macOS/Linux说明照搬。参考本地 `AGENTS.md` 和 `cli/remote.py`。需要管理员/驱动/新增配对而未授权时记BLOCKED/NOT_RUN，不把路线错误当产品共同缺陷。

## 最少证据清单

每个候选失败的 `comparison` 建议为：`MATCH_BOTH / MATCH_GO / MATCH_PMD / RUST_ONLY_DIFFERENCE / CONFLICTING / NO_COMPARATOR / PRECONDITION_MISMATCH / NOT_COMPARED`。`MATCH_GO/PMD`必须说明另一个为什么不适用。

记录两边：源码/实际工具路径及hash、run/设备/状态/素材、精确命令和服务route、开始结束时间、退出/超时/取消、数据或可观察效果、清理/残留、原始证据路径和脱敏副本。复杂输入归一化规则写进diff说明，不编辑原始失败日志。维护者必须能凭同一fixture和最短三步命令复现。
