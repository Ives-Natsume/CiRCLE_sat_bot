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
   - 每次创建都会覆盖原有模板
 - report
   - 添加报告到模板
   - 在定时任务中提交到AMSAT
 
通过定时任务拉取AMSAT报告、获得太阳活动图和处理本地报告

## TODO

 - 呼号绑定
 - 更细粒度的传播图
 - 过境查询模块迁移与开放
 - `/v`命令一键QRV
 - 过境匹配
