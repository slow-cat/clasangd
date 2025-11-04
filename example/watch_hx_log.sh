#!/bin/sh
watch "grep -Po '\[clasangd\]\K(.*)' <(tail -n25 helix.log)"
