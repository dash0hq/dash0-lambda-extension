package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"time"

	"github.com/aws/aws-lambda-go/lambda"
	"go.opentelemetry.io/contrib/instrumentation/github.com/aws/aws-lambda-go/otellambda"
	"go.opentelemetry.io/contrib/instrumentation/net/http/otelhttp"
	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/exporters/otlp/otlpmetric/otlpmetrichttp"
	"go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp"
	"go.opentelemetry.io/otel/metric"
	"go.opentelemetry.io/otel/propagation"
	sdkmetric "go.opentelemetry.io/otel/sdk/metric"
	"go.opentelemetry.io/otel/sdk/resource"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
	semconv "go.opentelemetry.io/otel/semconv/v1.37.0"
)

const extensionEndpoint = "http://127.0.0.1:9009"

type response struct {
	StatusCode int    `json:"statusCode"`
	Body       string `json:"body"`
}

type flusher struct {
	tracerProvider *sdktrace.TracerProvider
	meterProvider  *sdkmetric.MeterProvider
}

func (f flusher) ForceFlush(ctx context.Context) error {
	traceErr := f.tracerProvider.ForceFlush(ctx)
	metricErr := f.meterProvider.ForceFlush(ctx)
	log.Println("[tracing] forceFlush complete")
	if traceErr != nil {
		return traceErr
	}
	return metricErr
}

func newHandler(httpClient *http.Client, invocations metric.Int64Counter) func(context.Context, json.RawMessage) (response, error) {
	return func(ctx context.Context, event json.RawMessage) (response, error) {
		log.Printf("[handler] received event %s", event)
		invocations.Add(ctx, 1)

		req, err := http.NewRequestWithContext(ctx, http.MethodPost, "https://jsonplaceholder.typicode.com/posts", bytes.NewBufferString(`{"title":"go-manual"}`))
		if err != nil {
			return response{}, err
		}
		req.Header.Set("Content-Type", "application/json")
		resp, err := httpClient.Do(req)
		if err != nil {
			return response{}, err
		}
		defer resp.Body.Close()
		_, _ = io.Copy(io.Discard, resp.Body)
		log.Printf("[handler] outbound POST returned %d", resp.StatusCode)

		return response{StatusCode: 200, Body: `{"message":"Success"}`}, nil
	}
}

func main() {
	ctx := context.Background()

	res, err := resource.Merge(resource.Default(), resource.NewSchemaless(
		semconv.ServiceName(os.Getenv("AWS_LAMBDA_FUNCTION_NAME")),
	))
	if err != nil {
		log.Fatalf("creating resource: %v", err)
	}

	traceExporter, err := otlptracehttp.New(ctx, otlptracehttp.WithEndpointURL(extensionEndpoint+"/v1/traces"))
	if err != nil {
		log.Fatalf("creating trace exporter: %v", err)
	}
	tracerProvider := sdktrace.NewTracerProvider(
		sdktrace.WithBatcher(traceExporter),
		sdktrace.WithResource(res),
	)
	otel.SetTracerProvider(tracerProvider)
	otel.SetTextMapPropagator(propagation.NewCompositeTextMapPropagator(propagation.TraceContext{}, propagation.Baggage{}))

	metricExporter, err := otlpmetrichttp.New(ctx, otlpmetrichttp.WithEndpointURL(extensionEndpoint+"/v1/metrics"))
	if err != nil {
		log.Fatalf("creating metric exporter: %v", err)
	}
	// The periodic reader never fires during a short invocation; ForceFlush at handler end exports.
	meterProvider := sdkmetric.NewMeterProvider(
		sdkmetric.WithReader(sdkmetric.NewPeriodicReader(metricExporter, sdkmetric.WithInterval(time.Hour))),
		sdkmetric.WithResource(res),
	)
	otel.SetMeterProvider(meterProvider)

	invocations, err := meterProvider.Meter("dash0.test.go-manual").Int64Counter(
		"dash0.test.go_manual_invocations",
		metric.WithDescription("Number of invocations of the Go manual-instrumentation test function"),
	)
	if err != nil {
		log.Fatalf("creating counter: %v", err)
	}

	httpClient := &http.Client{
		Timeout:   8 * time.Second,
		Transport: otelhttp.NewTransport(http.DefaultTransport),
	}

	fmt.Println("[tracing] OpenTelemetry initialised")
	lambda.Start(otellambda.InstrumentHandler(
		newHandler(httpClient, invocations),
		otellambda.WithTracerProvider(tracerProvider),
		otellambda.WithFlusher(flusher{tracerProvider: tracerProvider, meterProvider: meterProvider}),
	))
}
