// Command vcon-client stores a vCon, finds it in the list, reads it back,
// and deletes it, with the Go standard library only.
//
// Set VCON_URL (https://vcon.example.com), VCON_KEY (a key with the write,
// read and delete scopes) and, for a certificate your system does not
// trust, VCON_CA (its PEM file).
package main

import (
	"bytes"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
)

type client struct {
	url, key string
	http     *http.Client
}

// call sends one request and returns the status and the body. An answer of
// 400 or more is a problem document, returned as an error with its type.
func (c client) call(method, path string, body []byte, contentType string) (int, []byte, error) {
	request, err := http.NewRequest(method, c.url+path, bytes.NewReader(body))
	if err != nil {
		return 0, nil, err
	}
	request.Header.Set("Authorization", "Bearer "+c.key)
	if contentType != "" {
		request.Header.Set("Content-Type", contentType)
	}
	answer, err := c.http.Do(request)
	if err != nil {
		return 0, nil, err
	}
	defer answer.Body.Close()
	read, err := io.ReadAll(answer.Body)
	if err != nil {
		return 0, nil, err
	}
	if answer.StatusCode >= 400 {
		var problem struct {
			Type string `json:"type"`
		}
		_ = json.Unmarshal(read, &problem)
		return answer.StatusCode, read, fmt.Errorf("%s %s: %d %s", method, path, answer.StatusCode, problem.Type)
	}
	return answer.StatusCode, read, nil
}

// newUUID is a random (version 4) uuid in RFC 9562 text form.
func newUUID() string {
	var b [16]byte
	rand.Read(b[:])
	b[6] = b[6]&0x0f | 0x40
	b[8] = b[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

func run() error {
	roots, err := x509.SystemCertPool()
	if err != nil {
		return err
	}
	if path := os.Getenv("VCON_CA"); path != "" {
		pem, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if !roots.AppendCertsFromPEM(pem) {
			return fmt.Errorf("no certificate in %s", path)
		}
	}
	c := client{
		url:  os.Getenv("VCON_URL"),
		key:  os.Getenv("VCON_KEY"),
		http: &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots}}},
	}
	id := newUUID()
	// encoding/json writes a map's keys sorted.
	vcon, err := json.Marshal(map[string]any{
		"created_at": "2026-10-25T12:00:00Z",
		"parties":    []any{},
		"subject":    "client example",
		"uuid":       id,
	})
	if err != nil {
		return err
	}

	// Store: 201, with the new version in ETag.
	status, _, err := c.call("POST", "/vcons", vcon, "application/vcon")
	if err != nil {
		return err
	}
	if status != 201 {
		return fmt.Errorf("a store answered %d, not 201", status)
	}
	fmt.Println("stored", id)

	// List: the newest vCon comes first.
	_, page, err := c.call("GET", "/vcons?limit=10", nil, "")
	if err != nil {
		return err
	}
	var listing struct {
		Vcons []struct {
			UUID string `json:"uuid"`
		} `json:"vcons"`
	}
	if err := json.Unmarshal(page, &listing); err != nil {
		return err
	}
	listed := false
	for _, v := range listing.Vcons {
		listed = listed || v.UUID == id
	}
	if !listed {
		return fmt.Errorf("the stored vCon is not in the list")
	}
	fmt.Println("listed", id)

	// Read: the body comes back exactly as stored.
	_, body, err := c.call("GET", "/vcons/"+id, nil, "")
	if err != nil {
		return err
	}
	if !bytes.Equal(body, vcon) {
		return fmt.Errorf("the body read differs from the one stored")
	}
	fmt.Println("read", id, len(body), "bytes")

	// Delete: 204, and the vCon is gone (410) from then on.
	status, _, err = c.call("DELETE", "/vcons/"+id, nil, "")
	if err != nil {
		return err
	}
	if status != 204 {
		return fmt.Errorf("a delete answered %d, not 204", status)
	}
	if status, _, _ := c.call("GET", "/vcons/"+id, nil, ""); status != 410 {
		return fmt.Errorf("a deleted vCon answered %d, not 410", status)
	}
	fmt.Println("deleted", id)
	return nil
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
