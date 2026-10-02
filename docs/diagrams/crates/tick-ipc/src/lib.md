# tick-ipc lib architecture

Source: `crates/tick-ipc/src/lib.rs`

```mermaid
flowchart TD
    A1["IpcFrame::new"] --> A2{"payload len <= 256"}
    A2 -- "no" --> X3["PayloadTooLarge code 3"]
    A2 -- "yes" --> A3["serialize field order"]
    A3 --> A4["TTIP 4B"] --> A5["version 1 2B"] --> A6["payload len 2B"] --> A7["token 32B"] --> A8["verb u32 4B"] --> A9["payload n B"] --> A10["crc32 trailer 4B"]

    A10 --> D0["deserialize"]
    D0 --> D1{"len >= 48"}
    D1 -- "no" --> X4["PayloadTruncated code 4"]
    D1 -- "yes" --> D2{"magic is TTIP"}
    D2 -- "no" --> X1["InvalidMagic code 1"]
    D2 -- "yes" --> D3{"version is 1"}
    D3 -- "no" --> X2["UnsupportedVersion code 2"]
    D3 -- "yes" --> D4{"len field <= 256"}
    D4 -- "no" --> X3
    D4 -- "yes" --> D5{"total len exact"}
    D5 -- "short" --> X4
    D5 -- "long" --> X15["TrailingBytes code 15"]
    D5 -- "exact" --> D6{"crc32 match"}
    D6 -- "no" --> X7["CrcMismatch code 7"]
    D6 -- "yes" --> D7{"verb maps 1 to 5"}
    D7 -- "no" --> X5["InvalidCommandVerb code 5"]
    D7 -- "yes" --> D8["IpcFrame token verb payload"]

    subgraph R["Response codec"]
        R1["encode_response"] --> R2["tag 1 Status utf8"]
        R1 --> R3["tag 2 Acquired u64 hns"]
        R1 --> R4["tag 3 Released no payload"]
        R1 --> R5["tag 4 Scheduled u32 id"]
        R1 --> R6["tag 5 Cancelled no payload"]
        R1 --> R7["tag 6 Error code and wire_param"]
        R2 --> R8["decode_response"]
        R3 --> R8
        R4 --> R8
        R5 --> R8
        R6 --> R8
        R7 --> R8
        R8 --> R9{"tag 1 to 6"}
        R9 -- "no" --> X11["MalformedResponse code 11"]
        R9 -- "yes" --> R10{"exact size and utf8"}
        R10 -- "no" --> X11
        R10 -- "yes" --> R11["IpcResponse variant"]
    end

    subgraph E["Error code map"]
        E0["IpcError::code"] --> E6["6 IntervalOutOfBounds"]
        E0 --> E8["8 UnauthorizedToken"]
        E0 --> E9["9 RateLimitExceeded"]
        E0 --> E10["10 MalformedToken"]
        E0 --> E12["12 TransportIo"]
        E0 --> E13["13 Unsupported"]
        E0 --> E14["14 ScheduleOutOfBounds"]
        E0 --> E16["16 ScheduleIdMismatch"]
    end

    subgraph C["Client exchange"]
        C1["connect retry loop"] --> C2["CreateFileW overlapped"]
        C2 -- "handle" --> C3["SetNamedPipeHandleState message mode"]
        C2 -- "ERROR_PIPE_BUSY" --> C4["WaitNamedPipeW busy budget"] --> C1
        C2 -- "other error" --> C5["io error out"]
        C3 --> C6["IpcClient ready"]
        C6 --> C7["exchange"]
        C7 --> C8["IpcFrame::new then serialize"]
        C8 --> C9["CreateEventW write"]
        C9 --> C10["WriteFile"]
        C10 -- "ERROR_IO_PENDING" --> C11["drain_overlapped"]
        C10 -- "immediate ok" --> C12["close write event"]
        C11 --> C13["WaitForSingleObject 5000 ms"]
        C13 -- "timeout" --> C14["CancelIoEx then GetOverlappedResult wait"] --> C15["close event TimedOut"]
        C13 -- "signaled" --> C16["GetOverlappedResult no wait"] --> C17["close event bytes written"]
        C12 --> C18{"short write"}
        C17 --> C18
        C18 -- "yes" --> C19["WriteZero error"]
        C18 -- "no" --> C20["read buffer 304 B"] --> C21["CreateEventW read"] --> C22["ReadFile"]
        C22 -- "ERROR_IO_PENDING" --> C23["drain_overlapped"]
        C22 -- "immediate ok" --> C24["close read event"]
        C23 --> C25["truncate to count"]
        C24 --> C25
        C25 --> C26["decode_response"]
    end
```

## Notes

* Frame header is 44 bytes so the smallest legal frame is 48 bytes with an empty payload plus the 4 byte CRC trailer.
* Payload length is capped at 256 bytes in both IpcFrame::new and deserialize, and an over long payload is code 3.
* CRC is standard CRC-32 with polynomial 0xEDB88320 and covers every header and payload byte before the little endian trailer.
* Decoding demands the exact total length, so short input is code 4 and extra input is code 15.
* Magic is TTIP, version is 1, and verbs map only 1 through 5, with violations reported as code 1, code 2 or code 5.
* Session tokens are 32 bytes, compared in constant time, and the Debug impl redacts the bytes instead of printing them.
* Token hex form is exactly 64 lowercase characters, so uppercase input and wrong lengths are rejected as code 10.
* Error codes are stable 1 through 16 and from_code_param returns None above 16, while secondary fields such as calculated come back zeroed.
* Interval validation accepts 5000 HNS or the inclusive 5000 to 156250 HNS band, anything else is code 6.
* Response tags run 1 through 6, only tag 1 takes variable length bytes, tags 3 and 5 require empty payloads, and tag 6 encodes code then wire_param, decoded via from_code_param.
* The client opens the pipe with FILE_FLAG_OVERLAPPED, so the write and the read each use a real OVERLAPPED with an event and every pended operation waits at most 5000 ms.
* On timeout the client cancels the I/O, drains the cancellation through GetOverlappedResult, then closes the event, and the rate limiter allows 10 requests per second per PID over at most 64 tracked PIDs.