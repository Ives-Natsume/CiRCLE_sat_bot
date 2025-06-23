from flask import Flask, request, jsonify

app = Flask(__name__)

@app.route('/', methods=['POST'])
def receive_message():
    try:
        data = request.get_json(force=True)
        print("Received POST request with JSON:")
        print(data)

        # 模拟响应
        return jsonify({
            "status": "ok",
            "echo": data
        }), 200

    except Exception as e:
        print("Failed to process request:", e)
        return jsonify({
            "status": "error",
            "message": str(e)
        }), 400

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=3300)
