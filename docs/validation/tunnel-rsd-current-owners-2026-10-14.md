# L02 当前 RSD 目录与原 lease 更新 · 2026-10-14

生产 alive 路径现在重新读取既定 route 的 checked RSD 握手，而非仅凭旧
ConnectedDevice cache 返回 NoChange。新的 immutable PreparedRsdCatalogue
在分配前按真实请求布局准入 String/Vec/body/selection，保存当前 UID、services、
ports 和 Features。SDK service resolution 优先使用当前目录；缺少 required
identity/Services/Port 时不发布 partial prefix，缺少 Features 与明确空数组区分。
原 rsd() 历史快照和 initial_rsd_probe() 原 cause/API 保持。

观察 proof 持原 key buffer identity/publication Arc、registry slot incarnation、
global/device stamps、runtime Arc、两组原费用及旧 fresh owner。它不创建或
Clone/Drop NativeLease，不替换 ManagedTunnel 或停止原 runtime。Source 前再次
核对 active exact ownership；原 serial、连接许可、authority/desired epoch 与
candidate/retry/whole 的绝对最短期限覆盖所有等待，没有重启 refresh timer。

新 catalogue/Metadata 的全局与 publication 费用准入时仍持有原两组费用和
旧目录。cache、table fresh owner 与 registry fresh owner 在 table→authority→
ticket→registry fences 内同步原位交换；晚 clock 恢复原 cache/table，registry
不变，旧/拒绝对象在 guard 外释放。最后 catalogue/table/native owner 持费直到
实际 cache removal 和 native joins 均完成，strict EOF/task baseline 保持。

每次 refresh 在 Source 前准备原 error capsule 和两份 report payload。Source
Ready(Err) 同一次 poll 即保存原 anyhow 对象并写入 registry/history 和 scan 的
独立 Freshness 槽，之后的期限、撤权、clear 或取消不丢原 cause，不增加 connect
attempt/backoff。downcast/source 保留原 typed error，没有 stringify 替换。新增
census snapshot 同池准入真实 KnownVec backing 后才 clone 原根；零额度时
保持 roots，报告 attribution incomplete。

新增17项（Core4/CLI13）覆盖 exact/backing/一字节退款、missing required fields、
真实 localhost H2/XPC 同地址变 ports 与实际新 service socket、生产 alive scan、
真实晚 swap 回滚、零额度/跨 clock 原 IO object、Source0 permit wait 取消、
device epoch/authority/new publication fences、原 absolute min deadline，以及
table removal/native joins 两种先后次序。CLI fixture 使用真实 userspace workers、
严格原 peer Ok(0) EOF 与 task baseline；device/current cache metadata 沿 cfgtest
seam 注入，没有物理设备或 host/remotectl discovery。

冻结572个构建源文件。首轮 Core prepared4/existing18/legacy34通过；CLI fixture
私有模块引用/借用析构6个编译错误的日志保留，改用既有 public test_support
和顺序 drop 后，freshness16/完整 tunnel498通过，0失败、2原ignored。随后
Clippy 仅作返回类型别名、Option::replace 和原 whole/session context 整理；
最终 affected freshness16复验通过，strict core/CLI、CLI build/fmt/diff通过。
no-default/tunnel-only Core check通过（8/44 warnings 保留，非严格 feature 全验）。
readonly manager1通过，Source/usbmux trap0；host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。其它平台/SDK制品/MSRV/feature组合保持原证据或
NOT_RUN。CLI SHA-256：
`7dc6fba43b6db560196007b06f2b828345715d3e7784299dd631b2a5d43f7c4d`。

证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-freshness`。
原 core/失败日志在父目录，完整 CLI 在 round2、core strict 在 round3，最终
affected/strict/build/check/smoke/受测 binary/source/raw Git proof 在 round4。
外部 original/proposed/guard/协议 source hashes 在相邻 l02-freshness-draft。
仅本地提交，无设备操作、push、发布或部署。

协议依据为 [go-ios pinned RSD](https://raw.githubusercontent.com/danielpaulus/go-ios/ca2c56d2c294ff702cadf4821edd30b069d3a264/ios/rsd.go)
与 [pymobiledevice3 pinned provider](https://raw.githubusercontent.com/doronz88/pymobiledevice3/03ba2270c0c80df77c37f65adc1967595970d31f/pymobiledevice3/remote/remote_service_discovery.py)。
它们读取当前握手的 UID/services/ports。由这些字段推断，相同 UID/ports 不能
证明远程 restart incarnation；本项没有生成虚构重启版本、设备 ACK 或物理认证。

完整 L02 仍未收口：raw H2/XPC tree/private scratch、原 Arc/anyhow/std IO/future
内部存储、SDK resolver 的返回副本、其它 native/channel/socket/runtime、
publication backing 与完整物理刷新/复用/重启规则仍需合同和交付。128MiB
不是完整 manager heap/RSS 上限；23交付/6缺开发/L02待收口、有效30与Q04
全部排除保持，本子项不升级主项。
