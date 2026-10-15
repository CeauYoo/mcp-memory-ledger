"""Model labels are provider assertions, never model-identity authentication."""
import re
from private_values import redact_known


def safe_model_name(value, credentials=()):
    if type(value) is not str or not 1 <= len(value) <= 80:
        return None
    if re.fullmatch(r'[A-Za-z][A-Za-z0-9]*(?:[-_.][A-Za-z0-9]+)*', value) is None:
        return None
    # Reject credential-like prefixes, opaque long runs and known key echoes.
    if (re.search(r'(?i)(?:^|[-_.])(?:sk|key|apikey|token|secret|password|bearer|authorization|credential)(?:$|[-_.])', value)
            or re.search(r'[A-Za-z0-9]{24,}', value)
            or value.startswith('eyJ')
            or redact_known(value, credentials) != value):
        return None
    return value


def model_metadata(value, requested, credentials=()):
    safe = safe_model_name(value, credentials)
    requested_safe = safe_model_name(requested, credentials)
    return {'requested_model': requested_safe, 'response_model': safe,
        'response_model_state': 'absent' if value is None else 'retained' if safe else 'withheld_unsafe',
        'provider_model_match': value == requested if type(value) is str else None,
        'model_name_source': 'provider_self_reported', 'model_identity_authenticated': False,
        'model_name_user_choice': 'not_required' if value == requested and safe else 'not_offered'}
