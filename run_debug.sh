#!/bin/bash
cargo run --release --bin car_simulation -p demo > debug.log 2>&1 &
PID=$!
sleep 3
kill -9 $PID
cat debug.log
