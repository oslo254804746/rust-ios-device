# L02 initial RSD owned production output · 2026-10-18

经理层 NativeSource 的 usbmux、direct USB、RemotePairing、TCP lockdown 四个
生产路由改用新增的 `*_owned_with_owner` Core 入口，返回 `OwnedConnectedDevice`。
Destination 在原 future/IO 前按实际输出类型大小准入。旧公开连接函数仍返回
ConnectedDevice 与原 mutable RsdHandshake；新输出私有持有实际连接，公开正确的
owned rsd()/into_rsd()、借用 info() 及原28项连接/服务/lockdown/freshness 能力。
没有可变 legacy unwrap、None cache 导出或事后目录复制适配。

私有栈上 InitialOwner 合并 cleanup owner 与原 bank，直接返回原 reservation Box，
不增加 wrapper 分配。拥有目录的策略在 transport 前检查 reservation 的同池身份，
activation 在启动/attach native worker 前再次取得并校验原池，attach 后不查询 bank。
Direct USB 建 tunnel 前读取 untrusted service 的 RSD 查询也使用 owned 原池输出。

初始 handshake 共用同一个四 mode 控制循环：Active/Queued/Legacy/Passive 使用
原 host UUID、fresh transport、dial/bootstrap/response 与外层期限；legacy 与 owned
仅选择不同 framer reader。六条 semantic 消息、wire 错误回退、ResourceLimit 立即
返回均保持。direct/proxy 的 owned 探测使用泛型 RsdProbeOutcome，保留 Available/
Failed/NotAttempted、原 Core/IO source 与无 mdns direct skip。目录与原 bank 通过
activation 原值移入连接。初始目录失败后，后续 owned retry 仍使用原池和原五次/
500ms 规则；已知 ResourceLimit 立即失败。

服务解析仍优先 current prepared catalogue，再使用 owned initial；不存在的服务
不会因空 legacy cache 重新探测，shim 仍先执行原 check-in/RemoteXPC 拒绝策略。
retained_metadata_bytes 将原 owned 目录实际 known_bytes 计入 published ceiling；
全局后置准入只扣除已由同一原池独立持有的实际外层 body 与目录 grant，避免重复
收费。不同池身份直接拒绝，不使用 legacy HashMap backing 估算替代 owned 目录。

本次新增8项 owned initial fixture，加上原SDK6项，共14项通过：
原 reservation Box 指针/同池/异池拒绝退款；四mode fresh fallback；六条耗尽后
Queued 新连接；真实 IPv6 localhost 建 tunnel 前目录查询；direct feature 前置；
公开 owned 输出/服务/metadata/into_rsd 导出与原指针寿命；Failed 原IO source；
实际 userspace activation 原 handle、原池在 attach 前取得、实际 shutdown joins/
任务基线与退款。原 proxy fixture 扩为三连接（legacy None、legacy 原池、owned
原池），验证20字节目的地址、稳定UUID、目录费用与最后 owner。

正向目录输出转移使用 scripted H2 + channel-only native handle，未启动 native
worker；实际 worker fixture 使用 duplex CDTunnel 和非法可选RSD endpoint，验证
Failed 路径的真实 joins。两者是各自独立证据，不声称正向真实设备端到端验证。
NativeSource 构造准入3项覆盖四个生产构造器，全部刻意不轮询真实 Source。

新跑 XPC210、activation8、proxy4、prepared4、existing18、Core tunnel131通过；
无 mdns owned13/proxy5通过，无默认及仅 tunnel 编译通过，cfg warnings 保留。
严格 Core/CLI all-features/all-targets Clippy -D warnings、完整CLI1731PASS/
0FAIL/13原ignored、build/fmt/diff通过。实际只读manager1通过，Source/usbmux trap0；
host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。Core 已修改，上述为本次
新验证；未执行其它平台/设备/SDK绑定/MSRV验证。

首轮严格编译发现测试泛型/新字段及参数数量，修正为明确类型、字段及私有栈参数
对象；第二轮修正公开入口位于测试模块之后的 lint，无 allow 抑制，日志保留。
第三轮 session 在完整CLI通过后构建时消失，进程检查确认无在途 Cargo/rustc/python；
没有 terminal result 的构建不计PASS，保存中断日志并在同一份源码上串行补完最后
五项。最终20项均有成功 result。679个冻结源码无漂移，21处源码差异已记录。
CLI SHA-256：
`82f221b31f2576ca0abb0295b7a654a8b4648b44839d30c991dde28363ee44e0`。
日志、binary、source delta、current 与提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-owned-initial-rsd-production\round3`。

这一交付接通经理层 initial RSD 原目录持有链；原 legacy SDK 目录、selected service
metadata String/Vec 拷贝及其它SDK输出仍是未计量边界。公开 prepared Arc/Weak body
寿命、private header/opaque IO/error/框架任务与allocator、完整 freshness 等继续审计。
128MiB是已准入已知对象池，非完整manager heap/RSS。L02不关闭，主项23交付/6缺开发/
L02待全审计、有效30与Q04全部排除保持。仅本地提交，无真实设备、push或发布。
