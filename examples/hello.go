// hello.go
package main

import (
	"encoding/json"
	"fmt"
	"os"
)

func main() {
	msg := map[string]any{
		"ok":  true,
		"msg": "hello world from Go (Normal and Tiny)",
	}

	data, err := json.Marshal(msg)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}

	fmt.Println(string(data))
}
