(component
  (import "wasi:io/error@0.2.6" (instance $errors
    (export "error" (type (sub resource)))))
  (alias export $errors "error" (type $error))
  (import "wasi:io/streams@0.2.6" (instance $streams
    (alias outer 1 $error (type $error))
    (export "output-stream" (type $output-stream (sub resource)))
    (type $stream-error-value (variant
      (case "last-operation-failed" (own $error)) (case "closed")))
    (export "stream-error" (type $stream-error (eq $stream-error-value)))
    (export "[method]output-stream.blocking-write-and-flush" (func
      (param "self" (borrow $output-stream))
      (param "contents" (list u8))
      (result (result (error $stream-error)))))))
  (alias export $streams "output-stream" (type $output-stream))
  (alias export $streams "[method]output-stream.blocking-write-and-flush" (func $write))
  (import "wasi:cli/stdout@0.2.6" (instance $stdout
    (alias outer 1 $output-stream (type $output-stream))
    (export "output-stream" (type $stream (eq $output-stream)))
    (export "get-stdout" (func (result (own $stream))))))
  (alias export $stdout "get-stdout" (func $get-stdout))
  (core module $data
    (memory (export "memory") 1)
    (data (i32.const 128) "{\22name\22:\22command\22,\22friendly_name\22:\22Command\22,\22description\22:\22CLI metadata\22,\22category\22:\22Testing\22,\22pins\22:[]}"))
  (core instance $data (instantiate $data))
  (alias core export $data "memory" (core memory $memory))
  (core func $get-stdout (canon lower (func $get-stdout)))
  (core func $write (canon lower (func $write) (memory $memory)))
  (core instance $host
    (export "get-stdout" (func $get-stdout))
    (export "write" (func $write)))
  (core module $main
    (import "host" "get-stdout" (func $stdout (result i32)))
    (import "host" "write" (func $write (param i32 i32 i32 i32)))
    (func (export "run") (result i32)
      call $stdout i32.const 128 i32.const 104 i32.const 0 call $write
      i32.const 0))
  (core instance $main (instantiate $main (with "host" (instance $host))))
  (alias core export $main "run" (core func $run))
  (func $run (result (result)) (canon lift (core func $run)))
  (instance $run (export "run" (func $run)))
  (export "wasi:cli/run@0.2.6" (instance $run)))
