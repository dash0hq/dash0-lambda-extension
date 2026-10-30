plugins {
    id("java")
}

repositories {
    mavenCentral()
}

java {
    toolchain { languageVersion = JavaLanguageVersion.of(17) }
}

dependencies {
    implementation("com.amazonaws:aws-lambda-java-core:1.2.3")
    implementation(platform("software.amazon.awssdk:bom:2.29.52"))
    implementation("software.amazon.awssdk:s3")
    implementation("software.amazon.awssdk:sqs")
    implementation("software.amazon.awssdk:url-connection-client")
}

configurations.all {
    exclude(group = "software.amazon.awssdk", module = "apache-client")
    exclude(group = "software.amazon.awssdk", module = "netty-nio-client")
}

tasks.register<Zip>("buildZip") {
    from(tasks.named("compileJava"))
    into("lib") {
        from(configurations.runtimeClasspath)
    }
}
