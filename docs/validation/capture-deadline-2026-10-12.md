# Q02 通用捕获晚到完成期限 · 2026-10-12

复核f75751d后发现：capture的while条件先检查leader/readers是否完成。如果启动或
调用者调度迟到，而原process和两个reader都已完成，控制循环可以一次也不进入，
跳过绝对deadline判断。原字节/exit0与host回收成功会被误当成预算内结果。

现在完成分支同样检查从capture入口开始的绝对期限；晚到完成保留原stdout/stderr、
退出码与cleanup事实，只将timed_out设为true，assess不能PASS。正常操作deadline与
独立host finally收尾保持，未增加终止范围、控制器输出或任何native/device ACK。

确定性测试使用真实child、原Job/process handle和原reader threads，仅控制宿主
调度：等待原handle退出与两个reader完成后才进入首次控制检查。修复前该测试失败，
观察timed_out=false；修复后通过。首个只等reader的夹具仍可能在process完成前进入
循环，原实现当时正确拒绝，因此其初始PASS仅保留为排程诊断，不冒充负控制。
原stdout=b"safe"、stderr=b"late-safe"、exit0/cleanup_complete与reader joins断言保持。

受影响组合110方法：105 PASS / 5平台skip；包含generic Windows tree/agent、watch
实际TLS startup Ctrl-C、AFC/XCTest/crash/WDA/RSD/Install observer与readonly manager
实际CLI gate。主机runner5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
Python语法/diff与source/log/commit关联另存summary。Rust本项未变、不重编译；沿用
f75751d产品来源的CLI，SHA256
`e9978f7eae758fe21337b0f2f5ad8dd1b923f88dd2a219dcbddab3bf2714c39e`。
这不是完整Rust/SDK/设备/其它平台重跑。

账本保持23开发交付/6缺开发/L02待完整收口，有效30，Q04全部排除。下一步回到L02
原error/history与其它metadata的字节归属/准入/最后释放；全局128MiB池的有限scope
和opaque未计量边界见[共享额度](tunnel-global-known-metadata-2026-10-12.md)。
按每项验证后立即本地提交执行，无子agent、push或设备操作。

外部证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12eff-9622-7f01-b777-335a6d20895d\repair\capture-deadline`。
入口`capture-deadline-summary.json`、`round-handoff.json`；原失败与诊断日志均保留。
