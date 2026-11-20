# Tasks For My Project

## build

> Builds my project

~~~sh
echo "building project..."
~~~

## test (file) [test_case]

> Run tests

~~~bash
echo "Testing $file ${test_case:-all}"
~~~

## serve

> Serve this directory

**OPTIONS**
* port
    * flags: -p --port
    * type: number
    * desc: Which port to serve on
* color
    * flags: --color
    * choices: RED, BLUE

~~~bash
[ "$verbose" = "true" ] && echo "Starting an http server on PORT: ${port:-8080}"
echo "port=${port:-8080} color=${color:-none}"
~~~

## services

> Commands related to starting and stopping services

### services start (service_name)

> Start a service.

~~~bash
echo "Starting service $service_name"
~~~

### stop (service_name)

> Stop a service.

~~~python
import os
print("Stopping service " + os.getenv("service_name"))
~~~

## ci

> Call other commands through $MASK

~~~bash
$MASK build && $MASK services start web && echo "dir=$(basename "$MASKFILE_DIR")"
~~~
