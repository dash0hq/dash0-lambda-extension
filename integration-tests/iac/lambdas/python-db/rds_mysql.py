import json
import os
import pymysql


def handler(event, context):
    conn = pymysql.connect(
        host=os.environ["DB_HOST"],
        port=int(os.environ.get("DB_PORT", "3306")),
        database=os.environ["DB_NAME"],
        user=os.environ["DB_USER"],
        password=os.environ["DB_PASSWORD"],
        ssl={"ssl": True},
    )

    try:
        with conn.cursor() as cur:
            cur.execute(
                """
                CREATE TABLE IF NOT EXISTS test_entries (
                    id INT AUTO_INCREMENT PRIMARY KEY,
                    request_id TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                )
                """
            )
            conn.commit()

            cur.execute(
                "INSERT INTO test_entries (request_id) VALUES (%s)",
                (context.aws_request_id,),
            )
            conn.commit()

            cur.execute(
                "SELECT * FROM test_entries WHERE request_id = %s",
                (context.aws_request_id,),
            )
            read = cur.fetchone()

        print(f"Inserted and read back entry for request: {context.aws_request_id}")

        return {
            "statusCode": 200,
            "body": json.dumps(
                {
                    "inserted": {"request_id": context.aws_request_id},
                    "read": str(read),
                }
            ),
        }
    finally:
        conn.close()
