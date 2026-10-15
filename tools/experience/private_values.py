"""Pure known-value redaction shared by local recording and display filters."""
import base64
import json
import re
from urllib.parse import quote


def variants(values, fragments=False):
    result = set()
    for value in values:
        if type(value) is not str or not value:
            continue
        data = value.encode('utf-8')
        standard = base64.b64encode(data).decode()
        urlsafe = base64.urlsafe_b64encode(data).decode()
        encoded_url = quote(value, safe='')
        forms = {value, encoded_url, re.sub(r'%[0-9A-F]{2}', lambda m:m.group().lower(), encoded_url),
                 data.hex(), data.hex().upper(), standard, standard.rstrip('='), urlsafe, urlsafe.rstrip('='),
                 json.dumps(value, ensure_ascii=True)[1:-1]}
        if value.startswith('sk-'):
            forms.add(value[3:])
        result.update(form for form in forms if form)
        if fragments:
            for form in forms:
                result.update(form[i:i+8] for i in range(max(0, len(form)-7)))
    return sorted(result, key=len, reverse=True)


def redact_known(text, values, fragments=True):
    patterns = variants(values, fragments)
    if not patterns:
        return text
    return re.sub('|'.join(re.escape(p) for p in patterns), '[REDACTED]', text,
                  flags=re.IGNORECASE)


def redact_bytes(raw, values):
    count = 0
    for form in variants(values):
        for encoding in ('utf-8','utf-16-le','utf-16-be','utf-32-le','utf-32-be'):
            needle = form.encode(encoding)
            count += raw.count(needle)
            raw = raw.replace(needle, '[REDACTED_CREDENTIAL]'.encode(encoding))
    # Also catch JSON strings with mixed/unicode escapes, without dropping any
    # surrounding fields, whitespace or even an otherwise malformed response.
    def json_string(match):
        nonlocal count
        try:
            value = json.loads(match.group())
            clean = value
            for form in variants(values):
                count += clean.count(form)
                clean = clean.replace(form, '[REDACTED_CREDENTIAL]')
            return json.dumps(clean, ensure_ascii=False).encode('utf-8') if clean != value else match.group()
        except (ValueError, UnicodeError):
            return match.group()
    raw = re.sub(rb'"(?:[^"\\]|\\.)*"', json_string, raw)
    return raw, count
