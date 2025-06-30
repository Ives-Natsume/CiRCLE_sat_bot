import requests
import json

url = "http://127.0.0.1:3300/query"

# 构造模拟 POST 请求体
payload = {
    "sender": {
        "user_id": 114514
    },
    "group_id": 123456,
    "message": "/query icm"
}

# 发送 POST 请求
response = requests.post(url, json=payload)

# 打印状态码与返回的 JSON 内容
print(f"Status Code: {response.status_code}")
try:
    print("Response JSON:")
    print(json.dumps(response.json(), indent=2, ensure_ascii=False))
except Exception as e:
    print("Failed to decode JSON:", e)
    print("Raw response:")
    print(response.text)
