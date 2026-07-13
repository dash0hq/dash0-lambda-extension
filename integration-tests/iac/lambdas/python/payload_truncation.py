def handler(event, context):
    # Worst-case mode (event is a JSON array): return ~5MB made of 280
    # strings of 18KB each. Replacing all of them lands just under the
    # default 4KB DASH0_MAX_EVENT_PAYLOAD, so the extension must replace
    # every string — the maximum-work path of JSON-aware truncation.
    if isinstance(event, list):
        return {
            'statusCode': 200,
            'items': ['y' * 18000] * 280,
        }

    # Default mode: return a payload larger than the default
    # DASH0_MAX_EVENT_PAYLOAD (4KB) so the extension has to truncate the
    # captured return value.
    response_size = int(event.get('response_size', 25000))
    return {
        'statusCode': 200,
        'small': 'keep-me',
        'password': 'response-secret',
        'big': 'y' * response_size,
    }
