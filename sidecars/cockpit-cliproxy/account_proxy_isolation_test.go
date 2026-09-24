package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

func TestAccountProxyIndependentTransports(t *testing.T) {
	var leaked atomic.Int32
	origin := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { leaked.Add(1); _, _ = io.WriteString(w, "DIRECT") }))
	defer origin.Close()
	proxyA := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { _, _ = io.WriteString(w, "A") }))
	defer proxyA.Close()
	proxyB := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { _, _ = io.WriteString(w, "B") }))
	defer proxyB.Close()
	for _, item := range []struct{ proxy, want string }{{proxyA.URL, "A"}, {proxyB.URL, "B"}, {proxyA.URL, "A"}} {
		client := providerGatewayHTTPClient(item.proxy)
		client.Timeout = time.Second
		response, err := client.Get(origin.URL)
		if err != nil {
			t.Fatal(err)
		}
		body, _ := io.ReadAll(response.Body)
		response.Body.Close()
		if string(body) != item.want {
			t.Fatalf("cross-account route: got %q want %q", body, item.want)
		}
	}
	if leaked.Load() != 0 {
		t.Fatal("request reached direct origin")
	}
	proxyA.Close()
	client := providerGatewayHTTPClient(proxyA.URL)
	client.Timeout = time.Second
	if response, err := client.Get(origin.URL); err == nil {
		response.Body.Close()
		t.Fatal("unreachable proxy fell back to direct")
	}
	if leaked.Load() != 0 {
		t.Fatal("failed proxy leaked direct traffic")
	}
}

func TestInvalidAccountProxyFailsClosed(t *testing.T) {
	client := providerGatewayHTTPClient("unsupported://secret@invalid")
	client.Timeout = time.Second
	if response, err := client.Get("http://127.0.0.1:1"); err == nil {
		response.Body.Close()
		t.Fatal("invalid proxy was not blocked")
	}
	if _, ok := client.Transport.(blockedProxyTransport); !ok {
		t.Fatal("invalid proxy returned a default transport")
	}
}
