import mysql from 'mysql2/promise';

export async function handler(event, context) {
    const connection = await mysql.createConnection({
        host: process.env.DB_HOST,
        port: parseInt(process.env.DB_PORT || '3306'),
        database: process.env.DB_NAME,
        user: process.env.DB_USER,
        password: process.env.DB_PASSWORD,
        ssl: { rejectUnauthorized: false },
    });

    try {
        await connection.execute(`
            CREATE TABLE IF NOT EXISTS test_entries (
                id INT AUTO_INCREMENT PRIMARY KEY,
                request_id TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        `);

        const [insertResult] = await connection.execute(
            'INSERT INTO test_entries (request_id) VALUES (?)',
            [context.awsRequestId],
        );

        const [rows] = await connection.execute(
            'SELECT * FROM test_entries WHERE request_id = ?',
            [context.awsRequestId],
        );

        console.log(`Inserted and read back entry for request: ${context.awsRequestId}`);

        return {
            statusCode: 200,
            body: JSON.stringify({
                inserted: { id: insertResult.insertId, request_id: context.awsRequestId },
                read: rows[0],
            }),
        };
    } finally {
        await connection.end();
    }
}
