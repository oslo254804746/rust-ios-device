# L02 原错误载荷采样与归因 · 2026-10-12

本项在既有原错误保留合同上补充只读计量与生产 debug 接线，不关闭 L02 主项。
Q02 已于736e575计开发完成；当前账本仍23交付、6缺开发、L02待完整收口，有效30，
Q04及全部子项排除。无子agent、推送、发布或设备操作；Rust/Cargo门禁串行。

## 实现与准确边界

`error_payload` 对 manager 的 retry `FailureReceipt.first/latest`、retired receipts、
abandoned scan 原错误及 native registry/current shutdown reports 做采样联合遍历。
每类 owner 在短锁内 clone 原引用，锁外检查；不调用 Display/Debug 格式化，不复制
任意错误文本，不更换原错误对象、source链、分类或receipt。原 buffer指针只在本次
RAM计量中去重，不入日志；全部根保持存活，避免释放后地址复用造成误去重。

已知 Core/Mux/Lockdown/PairRecord/Tunnel/XPC String与anyhow String按capacity计量，
涵盖独立Source/cleanup上下文；同一 first/latest 原对象和不同report共享的缓冲区只计一次。
循环链去重，深度32、节点65536上限；达到上限或加法溢出显式标记 truncated。
生产 monitor 仅在 DEBUG启用时输出固定数值/布尔字段，无身份/任意payload。

原 JoinHandle 尚未join时可持有未进入 ledger的Err/panic；计量不偷偷join或stop。
`unjoined_native_owners`与`active_scans`都使完整标志为false。原native joins之后，
已知buffer进入可观察范围；registry历史继续保留原错误，即使已回收所有运行任务。

不透明dyn Error/JoinError panic/原生库内部heap/PathBuf、anyhow/context与shutdown
report未知backing均明确计为opaque。已知字节是下界，opaque数量不是字节估计；
`fully_measured_payloads`仅表示本次已取样的payload没有这些未计量标志。
它不是总heap/RSS或所有manager元数据计量，也不是全局原子准入或字节上限。
采样期间新错误或其它子系统的改变仍可发生，不能把它用于并发准入决策。

## 验证与失败保留

新增8项Rust门禁覆盖：capacity与原对象身份、共享根去重、1MiB不透明payload且
格式化会panic的原对象、anyhow独立cleanup/String、循环/深链、三源scan弃置、
active scan未计量、生产first/latest/detach释放，以及真实userspace worker原IO故障、
原join、strict peer EOF、原任务baseline恢复与历史保留。
初次测试编译E0061是fixture调用私有构造器参数错误，修正后7通过/1失败；native
用例误把“已返回未join”当ledger已收集，发现并补上上述未join标志后8项通过。
初次严格Clippy有2处测试io_other_error，改成std::io::Error::other后重新检查。
所有失败日志保留，没有用计数、Drop、RST或强制回收替代原收尾断言。

受影响tunnel整组：359通过、2原有ignored（含新增8项）；不是全量Rust/SDK重跑。
两个ignored分别是Playwright/Chromium browser和显式L04测量；没有重跑或改成PASS。
Clippy要求修正两处新fixture的IO构造写法后，只重跑新增8项；未变生产代码与其它
351个tunnel方法按影响复用整组证据。严格Clippy、CLI build及实际readonly-policy
manager debug/shutdown/secret门禁已成功；fmt/语法/diff通过。
source/binary/log hashes和精确提交关联由外部summary记录。
实际CLI策略仅授予list/manage_server，生产权限检查跳过Source发现；localhost
usbmux trap另检查零连接。实际CLI首轮readiness失败来自fixture bearer格式不符合
生产64位小写hex合同，改成正式token格式后通过；原权限/readiness/raw扫描断言保持。
没有物理设备、mDNS、配对、安装、恢复或系统TUN操作。

下一步：把已知元数据和原error payload的准入/共享归属/最后释放接入统一全局规则；
不透明源必须有真实可测来源合同或明确未计量边界，不能用固定槽位数关闭L02。
随后按原refresh/reuse/restart/manual epoch及其它未交付主项继续。每个可验收项
验证后立即本地提交；本项采样交付不能替代尚缺的全局字节准入实现。

外部证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12eff-9622-7f01-b777-335a6d20895d\repair\l02`。
入口 `error-payload-summary.json`、`round-handoff.json`；旧精确CLI和失败制品均保留。
