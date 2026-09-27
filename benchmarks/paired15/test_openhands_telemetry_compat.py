"""Representative DeepSeek usage details survive SDK telemetry on the pinned baseline."""
# Run with the pinned Python 3.12: /home/sandbox/.local/share/uv/python/cpython-3.12.14-linux-x86_64-gnu/bin/python3.12
import pathlib,sys
R=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(R/'openhands-env12/lib/python3.12/site-packages'))
from litellm.types.utils import Usage
from openhands.sdk.llm.utils.telemetry import normalize_usage
for details,read,write in [({'cached_tokens':0},0,0),({'cached_tokens':3,'cache_creation_tokens':7},3,7),({'cached_tokens':3,'cache_write_tokens':9},3,9)]:
 u=Usage(prompt_tokens=20,completion_tokens=5,prompt_tokens_details=details)
 v=normalize_usage(u)
 assert (v.prompt_tokens,v.completion_tokens,v.cache_read_tokens,v.cache_write_tokens)==(20,5,read,write)
print('absent and present cache write details normalized, token accounting retained')
