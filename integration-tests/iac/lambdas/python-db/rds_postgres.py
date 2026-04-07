import json
import os
import psycopg2


def handler(event, context):
    conn = psycopg2.connect(
        host=os.environ["DB_HOST"],
        port=int(os.environ.get("DB_PORT", "5432")),
        dbname=os.environ["DB_NAME"],
        user=os.environ["DB_USER"],
        password=os.environ["DB_PASSWORD"],
        sslmode="require",
    )

    try:
        with conn.cursor() as cur:
            cur.execute(
                """
                CREATE TABLE IF NOT EXISTS test_entries (
                    id SERIAL PRIMARY KEY,
                    request_id TEXT NOT NULL,
                    created_at TIMESTAMPTZ DEFAULT NOW()
                )
                """
            )
            conn.commit()

            cur.execute(
                "INSERT INTO test_entries (request_id) VALUES (%s) RETURNING *",
                (context.aws_request_id,),
            )
            inserted = cur.fetchone()
            conn.commit()

            cur.execute(
                "SELECT * FROM test_entries WHERE request_id = %s",
                (context.aws_request_id,),
            )
            read = cur.fetchone()

        print(f"just a change!!! Inserted and read back entry for request: {context.aws_request_id}")

        return {
            "statusCode": 200,
            "body": json.dumps(
                {
                    "inserted": str(inserted),
                    "read": str(read),
                }
            ),
        }
    finally:
        conn.close()
