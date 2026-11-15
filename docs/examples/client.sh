# Source this file from Bash. Set token_file to the private file delivered by serve.
# api_base may be changed to another configured loopback URL or the HTTPS frontend.
api_base=${api_base:-http://127.0.0.1:8711}
kc() {
  test -r "$token_file" || return 1
  { printf 'Authorization: Bearer '; cat "$token_file"; printf '\n'; } |
    curl --disable --noproxy '127.0.0.1,::1,localhost' --silent --show-error --connect-timeout 3 --max-time 10 --header @- "$@"
}
