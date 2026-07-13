def handler(event, context):
    # Worst-case mode (event is a JSON array): return ~5MB made of ~1400
    # strings of 3.6KB each. Replacing all of them lands just under the
    # default 20KB DASH0_MAX_EVENT_PAYLOAD, so the extension must replace
    # nearly every string — the maximum-work path of JSON-aware truncation.
    if isinstance(event, list):
        return {
            'statusCode': 200,
            'items': ['y' * 3600] * 1400,
        }

    # Default mode: return a payload larger than the default
    # DASH0_MAX_EVENT_PAYLOAD (20KB) so the extension has to truncate the
    # captured return value.
    response_size = int(event.get('response_size', 25000))
    return {
        'statusCode': 200,
        'small': 'keep-me',
        'password': 'response-secret',
        'big': 'y' * response_size,
    }
