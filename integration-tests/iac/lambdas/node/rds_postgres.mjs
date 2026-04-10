import { Client } from 'pg';

export async function handler(event, context) {
    const client = new Client({
        host: process.env.DB_HOST,
        port: parseInt(process.env.DB_PORT || '5432'),
        database: process.env.DB_NAME,
        user: process.env.DB_USER,
        password: process.env.DB_PASSWORD,
        ssl: { rejectUnauthorized: false },
    });

    await client.connect();

    try {
        await client.query(`
            CREATE TABLE IF NOT EXISTS test_entries (
                id SERIAL PRIMARY KEY,
                request_id TEXT NOT NULL,
                created_at TIMESTAMPTZ DEFAULT NOW()
            )
        `);

        const insertResult = await client.query(
            'INSERT INTO test_entries (request_id) VALUES ($1) RETURNING *',
            [context.awsRequestId],
        );

        const readResult = await client.query(
            'SELECT * FROM test_entries WHERE request_id = $1',
            [context.awsRequestId],
        );

        console.log(`Inserted and read back entry for request: ${context.awsRequestId}`);

        return {
            statusCode: 200,
            body: JSON.stringify({
                inserted: insertResult.rows[0],
                read: readResult.rows[0],
            }),
        };
    } finally {
        await client.end();
    }
}
