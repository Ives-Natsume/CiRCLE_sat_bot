import requests

url = "http://localhost:3300"
payload = {
    "sender": {"user_id": 123456},
    "message": "/query so125",
    "group_id": 789012,
}

response = requests.post(url, json=payload)
print(f"状态码: {response.status_code}")
print(f"响应内容: {response.text}")