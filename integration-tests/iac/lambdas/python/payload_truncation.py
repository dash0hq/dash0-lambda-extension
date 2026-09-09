def handler(event, context):
    # Worst-case mode (event is a JSON array): return ~4.7MB made of 70k
    # strings of 64 bytes each. Replacing all of them lands under the
    # default 1MB DASH0_MAX_EVENT_PAYLOAD (70k * 14 bytes ~= 980KB), so
    # JSON-aware truncation is feasible and the extension must replace
    # ~69k of them — close to the most replacements it can ever perform
    # within a 1MB limit.
    if isinstance(event, list):
        return {
            'statusCode': 200,
            'items': ['y' * 64] * 70000,
        }

    # Default mode: return a payload larger than the default
    # DASH0_MAX_EVENT_PAYLOAD (1MB) so the extension has to truncate the
    # captured return value.
    response_size = int(event.get('response_size', 1_100_000))
    return {
        'statusCode': 200,
        'small': 'keep-me',
        'password': 'response-secret',
        'big': 'y' * response_size,
    }
