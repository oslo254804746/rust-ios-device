# L02 owned initial RSD SDK output · 2026-10-18

新增公开 `OwnedRsdHandshake`、`OwnedRsdService`、`OwnedRsdServices` 和
`owned_handshake_on_framer`。`H2Framer::connect_with_bank` 及 framer/error 类型
从 crate 根可用。调用者把原 byte bank 传给 framer，并负责已有 bootstrap；
owned 读取不创建新池、连接、bootstrap 或 deadline。原 legacy 公开字段、Clone、
ConnectedDevice 的 rsd()/into_rsd() 和连接入口保持原行为。

固定 Directory body 在读取前同池准入；两遍解析先计算全部有效服务的实际 Row、
Text、features Vec layout 和 UTF-8 字节，完整字段 grant 在第一处分配/复制前取得。
原 exact_vec 分配路径提取为校验 bank/完整 layout 的 grant helper，没有猜测
HashMap backing。无 scratch 的排序及去重保留最后有效重复服务，实际较大的 rows
capacity 仍付费，删除项原字段随析构退款。解析保留 legacy 空 UID、零/最大/字符串
port、混合/空/重复 features、缺失/错误 Services 的容忍规则，不替换成 strict refresh
parser。六条 semantic 消息上限、原 wire error、取消后 framing 和 ResourceLimit
立即返回规则保持。

目录、Clone、服务和 consuming iterator 持有同一私有 immutable root，借用原
字符串；无 raw Arc/Weak、可变 buffer 或裸拥有 buffer 导出。每个 clone Drop 均走
Arc::into_inner，使无 Weak 的 allocation 结束后才销毁返回的原数据及 permits。
这只覆盖已明确的 body/buffer layout；Arc 私有 header、allocator 与 opaque IO
不据此计量。known_bytes 是本对象已准入的实际已知布局，不是 RSS。

SDK 调用在启用 tunnel 后使用 crate 根入口：

```rust,ignore
let mut framer = ios_core::H2Framer::connect_with_bank(stream, Some(&original_bank)).await?;
// 在同一 framer 上完成调用者所需的 RSD bootstrap。
let directory = ios_core::owned_handshake_on_framer(&mut framer).await?;
let service = directory.get_resolved_service("com.apple.example");
// service/into_services()/Clone 可独立保留原目录及费用。
```

新增6项定向测试覆盖 legacy 差分/错误优先级、完整字段 grant 在首次分配前取得、
少1字节零输出分配、原字符串指针/服务/iterator 导出在输入和 framer 释放后存活、
8线程最后 owner、六条消息与关闭原池在下一次读取前拒绝。all-features 和无 mdns
均6项通过。XPC207、prepared4、existing18、Core tunnel131、无 mdns proxy/direct4
通过。CLI tunnel751/0FAIL/2原ignored、upstream10、完整CLI1731/0FAIL/13原ignored
通过。严格 all-features/all-targets Core/CLI Clippy -D warnings、build/fmt/diff 通过；
无默认及仅 tunnel 编译通过，实际 cfg warnings 保留。实际只读 manager1 通过，
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。

首轮测试借用销毁顺序编译失败已修正；公开入口也补齐 crate 根导出，没有以
allow dead_code 隐藏不可达 API。失败日志保留。第二轮19个串行门禁全部通过，
677个冻结源码无漂移，6处源码差异；本次 Core 已修改，上述 Core 验证为本次新跑。
CLI SHA-256：
`17bf2de6913db395e3e624907893baadc4213de7d8bd3698f349f18b597b6e10`。
日志、source delta、受测 binary、current 和提交后 raw Git proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-owned-initial-rsd-sdk\round2`。

经理层 NativeSource/四种连接入口/activation/direct/proxy/四 mode 的 owned 输出接入
仍 PENDING。本 SDK framer API 是可用的协议入口，本项不关闭经理层原持久目录缺口；
设备侧 sidecar、None cache 或事后 legacy HashMap 容量估算不作为替代。建 tunnel
之前 direct USB 的 RSD 读取也需覆盖。公开 prepared root/Weak、SDK 输出副本、错误/
private header/框架 IO/task/allocator 和完整 freshness 继续审计。128MiB 为已准入
已知对象池，非完整 manager heap/RSS；L02不关闭，主项23交付/6缺开发/L02待全审计、
有效30及Q04全部排除保持。仅本地提交，无真实设备、push或发布。
