# CiRCLE_sat_bot_server

重构后成为了[rinko_bot_core](https://github.com/Ives-Natsume/rinko_bot_core)的后端，力求实现更高的鲁棒性，目前作为一个独立服务存在

正在逐步迁移原有功能，并且正在添加新的功能

目前已经在大群部署

目前主要支持以下命令：
 - q
    - 查询AMSAT中的卫星状态
 - s
    - 获得当前太阳活动图
 - create
   - 为某次过境创建报告模板
 - report
   - 缓存报告到模板
   - 过境结束20分钟后提交到AMSAT
   - 本地缓存报告冲突检查
 - r (roaming)
   - 用于上传和共享群友的漫游信息

## TODO

 - 呼号绑定
 - 更细粒度的传播图
 - 过境查询模块迁移与开放
 - 过境匹配
 - 剩下的忘了，想起来再说，有建议欢迎issue

## 鸣谢

BA8AFK的[过境查询模块](https://github.com/AwayFromBiscuits/SatPassPredictAPI)，感谢小萝莉

感谢CHDHAM，感谢所有承担测试工程师的，乱输命令的群友

爱来自Rinko
