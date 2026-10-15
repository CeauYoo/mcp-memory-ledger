"""Private full diagnostics on the user's Mac. Never print these records."""
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit
from private_values import redact_bytes

CODE_ROOT = Path(__file__).resolve().parent
CODE_FILES = ('run.py','catalog.py','context_material.py','ledger.py','protocol.py','network.py','reporting.py',
              'support.py','model_names.py','private_values.py','local_records.py','test_experience.py',
              'answer_checks.py','staged.py','live_monitor.py','test_staged.py','Start-Retest.command','README.txt','AGENTS.md')


def credential_field(name):
    return bool(re.search(r'(?i)auth|cookie|token|secret|password|api[-_]?key|session|csrf|^key$', name))


def clean_text(value, credentials):
    raw, count = redact_bytes(str(value).encode('utf-8'), credentials)
    return raw.decode('utf-8'), count


def clean_url(value, credentials):
    parts = urlsplit(value)
    authority = parts.netloc.rsplit('@', 1)[-1]
    query = urlencode([(key, '[REDACTED_CREDENTIAL]' if credential_field(key) else val)
                       for key, val in parse_qsl(parts.query, keep_blank_values=True)])
    return clean_text(urlunsplit((parts.scheme, authority, parts.path, query, parts.fragment)), credentials)[0]


def filtered_headers(headers, credentials):
    kept, omitted, count = [], 0, 0
    for name, value in headers:
        if credential_field(name):
            omitted += 1
            continue
        name, n = clean_text(name, credentials)
        value, v = clean_text(value, credentials)
        if name.lower() in ('location', 'content-location'):
            value = clean_url(value, credentials)
        kept.append([name, value]); count += n + v
    return {'values': kept, 'credential_headers_omitted': omitted,
            'credential_replacements': count}


def body_record(raw, credentials, complete):
    # Scrub original bytes first: encoding heuristics must never hide a UTF-8
    # credential embedded after a binary prefix. Scrub normalized text again.
    scrubbed, raw_replacements = redact_bytes(raw, credentials)
    encoding = json.detect_encoding(scrubbed)
    try:
        normalized = scrubbed.decode(encoding).encode('utf-8')
        source_encoding = encoding
    except UnicodeError:
        normalized, source_encoding = scrubbed, 'binary_or_unknown'
    clean, replacements = redact_bytes(normalized, credentials)
    replacements += raw_replacements
    record = {'received_bytes': len(raw), 'body_complete': complete,
              'source_body_encoding': source_encoding,
              'credential_replacements': replacements,
              'redaction_policy': 'exact_known_credentials_and_common_encodings',
              'sanitized_body_sha256': hashlib.sha256(clean).hexdigest()}
    try:
        record['body_text_utf8'] = clean.decode('utf-8')
    except UnicodeError:
        record['body_base64'] = base64.b64encode(clean).decode('ascii')
    return record


def write_new(path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    data = json.dumps(value, ensure_ascii=False, indent=2).encode('utf-8') + b'\n'
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(data); stream.flush(); os.fsync(stream.fileno())


def capture_code(folder, extra_files=()):
    directory = folder/'code';directory.mkdir(mode=0o700)
    hashes = {}
    for name in dict.fromkeys((*CODE_FILES, *extra_files)):
        raw = (CODE_ROOT/name).read_bytes()
        hashes[name] = hashlib.sha256(raw).hexdigest()
        path = directory/name
        with path.open('xb') as stream:
            stream.write(raw)
        path.chmod(0o600)
    value = {'recorded_utc': datetime.now(timezone.utc).isoformat(), 'code_sha256': hashes,
             'revision': 'thirty-scenario-conversation-trial', 'local_only': True}
    write_new(folder/'code-manifest.json', value)
    return hashes


class CallRecords:
    def __init__(self, folder, ordinal, code_hashes, metadata=None):
        self.folder, self.ordinal, self.code_hashes = folder, ordinal, code_hashes
        self.metadata = metadata or {}

    def base(self):
        return {**self.metadata,'call_number': self.ordinal, 'run_id': self.folder.name,
                'recorded_utc': datetime.now(timezone.utc).isoformat(),
                'code_sha256': self.code_hashes, 'local_only': True}

    def request(self, url, raw, credentials, started):
        value = {**self.base(), 'attempt_started_utc': started,
                 'method': 'POST', 'url': clean_url(url, credentials),
                 'headers': [['Content-Type','application/json']],
                 'authorization_recorded': False, **body_record(raw, credentials, True)}
        write_new(self.folder/'diagnostics'/f'call-{self.ordinal:02d}.request.json', value)

    def response(self, status, headers, raw, credentials, complete, started, transport_status):
        value = {**self.base(), 'attempt_started_utc': started, 'http_status': status,
                 'transport_status': transport_status,
                 'headers': filtered_headers(headers, credentials),
                 **body_record(raw, credentials, complete)}
        write_new(self.folder/'diagnostics'/f'call-{self.ordinal:02d}.response.json', value)
