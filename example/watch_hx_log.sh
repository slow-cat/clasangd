#!/bin/sh
watch "grep -m 25 -Po '\[clasangd\]\K(.*)' <(tac < helix.log)"
