def handler(event, context):
    # Return a payload larger than the default DASH0_MAX_EVENT_PAYLOAD (20KB)
    # so the extension has to truncate the captured return value.
    response_size = int(event.get('response_size', 25000))
    return {
        'statusCode': 200,
        'small': 'keep-me',
        'password': 'response-secret',
        'big': 'y' * response_size,
    }
