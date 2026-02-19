plugins {
    id("java")
}

group = "org.example"
version = "1.0-SNAPSHOT"

repositories {
    mavenCentral()
}

dependencies {
    implementation("com.amazonaws:aws-lambda-java-core:1.2.3")
    testImplementation(platform("org.junit:junit-bom:5.10.0"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

tasks.test {
    useJUnitPlatform()
}

tasks.register<Zip>("buildZip") {
    from(tasks.named("compileJava"))
    from(tasks.named("processResources"))
    into("lib") {
        from(configurations.runtimeClasspath)
    }
}

tasks.named("build") {
    dependsOn("buildZip")
}