import { Client } from 'pg';
import { SecretsManagerClient, GetSecretValueCommand } from '@aws-sdk/client-secrets-manager';

const secretsManager = new SecretsManagerClient();

async function getCredentials() {
    const secretArn = process.env.DB_SECRET_ARN;
    const response = await secretsManager.send(new GetSecretValueCommand({ SecretId: secretArn }));
    return JSON.parse(response.SecretString);
}

export async function handler(event, context) {
    const credentials = await getCredentials();

    const client = new Client({
        host: process.env.DB_HOST,
        port: parseInt(process.env.DB_PORT || '5432'),
        database: process.env.DB_NAME,
        user: credentials.username,
        password: credentials.password,
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
