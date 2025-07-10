# CiRCLE_sat_bot

湿润千回的天空

其实作者是Rinko厨（笑）

## 功能列表

 - 查询指定AMSAT卫星状态
 - AMSAT卫星状态变更通知
 - 指定卫星过境预测
 - 卫星过境倒计时列表
 - 卫星过境倒计时提醒
 - 传播状态调用
 - 暂时没了

## 使用说明

直接在群聊中输入命令即可，也可以@Rinko使用

可用命令：

 - `/<query / q> <卫星id/卫星名/卫星别名>`，查询卫星在线状态
 - `/<pass / p> <sat_name>`，查询卫星过境预测
 - `/<all / a>`，查询所有卫星过境的倒计时列表
 - `/<sun / s>`，查询实时（应该）太阳图
 - `/add <卫星id>`，添加临时可查询卫星
 - `/del <卫星id/卫星名/卫星别名>`，删除临时可查询卫星
 - `/<permission / chmod> <卫星id/卫星名/卫星别名> <track = t（查询）/notify = n（通知）>`，开关卫星的查询和通知功能
 - `/<help / h>`，查询帮助文本
 - `/about`，查询bot相关信息

使用示例：

``` text
TX:
/q iss

RX:
ISS-DATA:
Status:
Report time: xx
Transponder/Repeater Active (1 reports)

ISS-FM:
Status:
Report time: xx
ISS Crew (Voice) Active (3 reports)
```

`/query`命令会返回所有符合查询条件的卫星
由于数据直接来源于AMSAT，所以AMSAT没有的，这里也没有

``` text
TX:
/p so50

RX:
SAUDISAT 1C 过境：起始 08-10 11:45，最高仰角 19.1°，结束 08-10 14:51，持续 191981 秒
```

`/pass`命令会返回手动选择的存储库内符合查询条件的卫星

## 项目结构

### AMSAT拉取/解析系统

定时任务，每隔一定时间拉取AMSAT的HTML文件并解析数据为json文件，以供本地查询

众所周知，AMSAT的每个小格子代表2h，所以在解析时需要计算当前处于第几个格子，在未来的格子需要跳过解析

### 卫星过境预测系统

定时任务，每隔一定时间向~~N2YO~~自建API发起API请求并解析存储为json文件，以供本地查询

自建API系统为基于Skyfield和FastAPI的卫星过境预测系统，在24小时内与Look4Sat的预测有1秒时间和0.2度方位角的误差，详见[github项目](https://github.com/AwayFromBiscuits/SatPassPredictAPI)

API服务由BA8AFK友情赞助，果然白毛红瞳小萝莉是无敌的

### 传播状态拉取系统

定时任务，每隔一段时间拉取hamqsl的传播状态图并本地存储（正在完善）后在需要时发送

### 热加载系统
引入一个临时卫星表用于高效储存和管理一些临时需要的卫星（如sstv类的短时开放卫星），同时对接了API以快速调用

### 查询系统

由于l4s，heavens above等软件对卫星名称的支持较差，导致有时候找卫星非常头疼
对于用户输入的各种缩写/别名，首先进行字符串预处理，再与映射表进行比对，完美实现对卫星别名的支持

例如，`Hades-ICM` `icm` `so125` `SO-125`或者什么奇奇怪怪的大小写组合，只要你写了`icm`或者`so125`就可以完成查询（不要写`Hades-ICM-so125`）

预处理后到之前拉取的json文件里查找就行

### Bot系统

使用LLOneBot，其实我还是没搞清楚OneBot协议该怎么用，反正最后是跑起来了

后续可能会迁移到NoneBot？

_It Just Work_
