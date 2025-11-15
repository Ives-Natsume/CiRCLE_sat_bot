# CiRCLE_sat_bot_server aka Rinko

[rinko_bot_core](https://github.com/Ives-Natsume/rinko_bot_core)的后端，力求实现更高的鲁棒性，目前作为一个独立服务存在，负责所有功能相关的模块，也就是rinko有时候会提到的“外部服务器” (其实都跑在一个机器里)

目前主要支持以下命令：
 - q
    - 查询AMSAT中的卫星状态
 - create
   - 为某次过境创建报告模板
 - report
   - 缓存报告到模板
   - 过境结束20分钟后提交到AMSAT
   - 本地缓存报告冲突检查

## 鸣谢

BA8AFK的[过境查询模块](https://github.com/AwayFromBiscuits/SatPassPredictAPI)，感谢小萝莉

PS: AFK写了个[过境匹配工具](https://satlover.de/satfind)

感谢CHDHAM，感谢所有承担测试工程师的，乱输命令的群友

爱来自Rinko
