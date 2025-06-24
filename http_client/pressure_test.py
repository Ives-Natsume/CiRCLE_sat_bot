import requests
import threading
import time
import random
import uuid

# 测试配置
BASE_URL = "http://localhost:3300/query"
NUM_USERS = 300
REQUEST_DELAY = 0.001  # 秒 (模拟用户输入时间差异)

def send_query(user_id, group_id):
    """模拟单个用户发送查询"""
    try:
        # 创建唯一消息ID跟踪请求
        message_id = str(uuid.uuid4())[:8]
        
        # 准备请求数据
        payload = {
            "sender": {"user_id": user_id},
            "message": f"/query iss",
            "group_id": group_id,
        }
        
        # 记录请求开始时间
        start_time = time.time()
        
        # 发送请求
        response = requests.post(BASE_URL, json=payload)
        
        # 计算响应时间
        duration = time.time() - start_time
        
        # 打印结果
        status = "Y" if response.status_code == 200 else "N"
        print(f"[用户 {user_id}] {status} 耗时: {duration:.3f}s | 消息: {payload['message']} | 状态码: {response.status_code}")
        
        # 打印详细错误信息（如果有）
        if response.status_code != 200:
            print(f"  错误详情: {response.text}")
    
    except Exception as e:
        print(f"[用户 {user_id}] ✗ 请求失败: {str(e)}")

def run_concurrency_test():
    """运行并发测试"""
    print(f"开始并发测试: {NUM_USERS} 个用户同时查询...")
    print("=" * 60)
    
    threads = []
    start_time = time.time()
    
    # 创建并启动所有线程
    for i in range(NUM_USERS):
        # 生成随机用户ID (100000-999999) 和群ID (10000-99999)
        user_id = random.randint(100000, 999999)
        group_id = random.randint(10000, 99999)
        
        # 创建线程
        t = threading.Thread(
            target=send_query, 
            args=(user_id, group_id),
            daemon=True
        )
        
        threads.append(t)
        t.start()
        
        # 添加随机延迟模拟真实用户行为
        time.sleep(random.uniform(0, REQUEST_DELAY))
    
    # 等待所有线程完成
    for t in threads:
        t.join()
    
    # 计算总耗时
    total_time = time.time() - start_time
    print("=" * 60)
    print(f"测试完成! 总耗时: {total_time:.2f} 秒")
    print(f"平均响应时间: {total_time/NUM_USERS:.3f} 秒/请求")

if __name__ == "__main__":
    run_concurrency_test()