Share a small semantic contract between two online apps while keeping ownership in the producer. Prerequisites: Data Studio tables, basic role administration and permission to manage connections in both practice apps. Use synthetic data.

Create `Parts Producer` and `Parts Consumer`, or use two disposable apps you administer. Import the supplied rows into a project table in the producer:

@PartsFixture
@ConnectionChecks

In the producer, create a connection role that grants the required data reading, such as `ReadDatabase`, without elevation or unrelated writes. Through **Team → Connections**, grant or approve access for the consumer app using that role. Verify the connection is active and points from consumer to producer. The producer controls the grant.

The person discovering contracts also needs Read Boards in the consumer. Installing/refreshing requires Write Files or Write Databases there. These local permissions and the producer connection role are separate checks.

Completion: record source app, target app, granted role and active status. Neither an app connection nor its role exposes every ontology automatically. The next lesson creates and exposes one contract. Reference: [remote ontologies](https://docs.flow-like.com/topics/ontology/remote/).
