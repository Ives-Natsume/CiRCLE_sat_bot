# CiRCLE_sat_bot

湿润1000回的天空

其实作者是Rinko厨（笑）

## 功能列表

 - 查询AMSAT卫星状态
 - AMSAT卫星状态变更通知
 - 暂时没了

## 使用说明

目前仅支持群内直接@使用，回复的@或者其他方式都不会响应

可用命令：

 - `/query <sat_name>`，`<sat_name>`对大小写和特殊符号不敏感，支持简称/别名
 - `/help` (`/h`)
 - `/about`

使用示例：

```
TX:
@Shirokane Rinko /query iss

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

## 项目结构

### AMSAT拉取/解析系统

定时任务，每隔一定时间拉取AMSAT的HTML文件，解析数据为json文件以供本地查询

众所周知，AMSAT的每个小格子代表2h，所以在解析时需要计算当前处于第几个格子，在未来的格子需要跳过解析

### 查询系统

由于l4s，heavens above等软件对卫星名称的支持较差，导致有时候找卫星非常头疼
对于用户输入的各种缩写/别名，首先进行字符串预处理，再与映射表进行比对，完美实现对卫星别名的支持

例如，`Hades-ICM` `icm` `so125` `SO-125`或者什么奇奇怪怪的大小写组合，只要你写到了`icm`或者`so125`就可以完成查询（不要写`Hades-ICM-so125`）

预处理后到之前拉取的json文件里查找就行

### Bot系统

使用LLOneBot，其实我还是没搞清楚OneBot协议该怎么用，反正最后是跑起来了

_It Just Works_
